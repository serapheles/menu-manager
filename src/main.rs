use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::{extract::{Path, State}, extract, http, response, Router, routing::get};
use axum::extract::MatchedPath;
use axum::http::{Request};
use serde_json::{json, Value};
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::{info_span, Span};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_panic::panic_hook;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::menu::{add_json_to_db, get_map, Item};

mod menu;

//Declaring where the database is, instead of determining and passing along like in Android/Crux,
//is so much easier it feels wrong. ;_;
static PATH: &str = "res/menu_db.sqlite";

//App itself should just read the json responses; allows adding fields on this (server) side without
//needing to update the app. However, that could complicate caching responses.

//State struct to have shared state across router functions.
//Allows local (app) access to the HashMap.
//Wrapped in an atomic reference counted read-write lock to allow async/multithreaded access.
//There are a couple aof points of uncertainty regarding how/when references are added/dropped, so
//I'm slightly concerned this may break if running and being used for a while... it's probably fine.
//Using the default example name because names are hard.
//Can't say this is my favorite pattern.
#[derive(Clone)]
struct AppState {
    map: Arc<RwLock<HashMap<String, HashSet<Arc<Item>>>>>,
}

//Initial setup, could/should implement something to avoid this going forward.
fn _db_load() {
    add_json_to_db(PATH, "res/bateau_04-11.json").unwrap();
    add_json_to_db(PATH, "res/canlis_06-03.json").unwrap();
    add_json_to_db(PATH, "res/lark_06-03.json").unwrap();
    add_json_to_db(PATH, "res/westward_05-16.json").unwrap();
}

// Ultra basic server setup (give or take the Arc<> stuff); we don't really need much beyond a basic
// query to find items.
// https://github.com/joelparkerhenderson/demo-rust-axum used as a starting point/guide.
//TODO: add a post end point to upload JSON formatted menus or individual items
#[tokio::main]
async fn main() {
    //Sets up a rolling log file.
    //There's a *lot* of components to the tracing logger, and they all had their own documentation,
    //but almost no clear examples as to how they fit together.
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("menu_manager.log")
        .build("res/")
        .expect("Log file should have been created. Check file paths.");

    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    //Set up the logging format layer.
    //By default, ANSI escape characters are included that are illegible in a text reader.
    let fmt_layer = fmt::layer()
        // .pretty()
        .with_ansi(false)
        .with_writer(non_blocking);
    //Set up the filter layer. Attempts to use the RUST_LOG env level, if it exists.
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("trace"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    //Logs panics
    std::panic::set_hook(Box::new(panic_hook));

    menu::ensure_db(PATH).expect("Database should have been created. Check for permissions.");
    _db_load();

    let state = AppState {
        map: Arc::new(RwLock::new(get_map(PATH))),
    };

    let app = Router::new()
        .fallback(
            fallback
        )
        // .route("/",
        //        get(|| async { "Hello, World!" }),
        // )
        .route("/query/:input",
               get(query),
        )
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    let matched_path = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);

                    info_span!(
                        "http_request",
                        method = ?request.method(),
                        matched_path,
                        some_other_field = tracing::field::Empty,
                    )
                })
                .on_request(|request: &Request<_>, _span: &Span| {
                    tracing::debug!("started {} {}", request.method(), request.uri().path())
                })
                //Not currently concerned with these options, but they exist.
                .on_response(())
                .on_body_chunk(())
                .on_eos(())
                //
                .on_failure(|error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {
                    tracing::error!("something went wrong: {}", error)
                }),
        )
        .with_state(state);

    // Run our application as a hyper server on http://localhost:3000.
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

//Simple handle that takes an input string, splits it into tokens by whitespace, and returns a JSON
//array of all the items found that include the passed tokens.
//Slightly worried about accessing the item via pointer, then cloning, and if that impacts the Arc.
async fn query(
    Path(mut input): Path<String>,
    State(state): State<AppState>,
) -> extract::Json<Value> {
    let mut res: HashSet<Item> = HashSet::new();

    input.retain(|x| x.is_alphabetic() || x.is_whitespace());
    for i in input.split(char::is_whitespace) {
        if let Some(x) = state.map.read()
            .expect("State HashMap should be available at this point.")
            .get(i) {
            for item in x {
                res.insert((**item).clone());
            }
        }
    }
    json!(res).into()
}

//Handler for calls to undefined routes.
//In real world situations, I feel like this would be important to monitor for security reasons.
async fn fallback(
    uri: http::Uri
) -> impl response::IntoResponse {
    //Is warn too high?
    tracing::warn!("Undefined route called: {}", uri);
    (http::StatusCode::NOT_FOUND, format!("No route {}", uri))
}