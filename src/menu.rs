use std::{fmt, fs};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::Formatter;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use rusqlite::{Connection, params, Result};
use serde::{Deserialize, Serialize};
use tracing::{trace, warn};

//Left as a reference
// pub(crate) const _WESTWARD_FILE: &[u8] = include_bytes!("../res/westward_05-16.json");
// pub(crate) const _BATEAU_FILE: &[u8] = include_bytes!("../res/bateau_04-11.json");

//Struct for menu items. Holds basic details as various fields.
//There are a number of points I'm not sure how I would like to handle, so I've largely opted for
//the simplest ways I haven't dismissed.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Item {
    item_name: String,
    ingredients: Vec<String>,
    updated: String,    //This should be a date, but that complicates a lot of things.
    price: String,      //This should probably be an enum, but I'm not sure how to divide options.
    restaurant: String, //This would probably benefit from being a struct.
}

//Basic functions.
impl Item {
    //Was used briefly for something. Leaving in case I need it later.
    pub fn _get_ingredients_str(&self) -> String {
        let mut ingredients = String::new();
        //I feel like this could be done using a map & collect?
        for i in &self.ingredients {
            ingredients.push_str(i.as_str());
            ingredients.push_str(", ");
        }
        ingredients
    }

    //Slightly modified from the example at https://doc.rust-lang.org/std/hash/index.html
    pub fn get_hash(&self) -> u64 {
        let mut s = DefaultHasher::new();
        self.hash(&mut s);
        s.finish()
    }
}

//Pretty print
//Will probably change this later.
impl fmt::Display for Item {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {:?}, {}", self.item_name, self.ingredients, self.price)
    }
}

//Allows items to be hashed for uniqueness while ignoring the updated date.
//Ignores price too, which could cause incongruities, but price is a bit more finicky and a lower personal concern.
//Should perhaps ignore ingredients, for the sake of updating existing entries.
//Alternatively, could have a separate method that returns a hash that excludes ingredients?
impl Hash for Item {
    //This looks pretty weird to me. I can make some guesses how this works, but honestly, when
    // looking through the source, there are some points I simply couldn't find.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.item_name.hash(state);
        self.ingredients.hash(state);
        self.restaurant.hash(state);
    }
}

//Checks if the database exists. If not, creates it.
//This should probably return something that lets me know if a database needed to be created.
//That said, what would the response be? Attempt to load every json file in res/?
pub(crate) fn ensure_db(path: &str) -> Result<(), Box<dyn Error>> {
    let connection = Connection::open(path)?;

    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='menu_db'",
    )?;

    let mut res = statement.query([])?;

    if (res.next()?).is_some() {
        //Only(?) does anything if a logger is set up and running
        //Notably, this means it doesn't show up in current tests.
        trace!("Database found.")
    } else {
        warn!("Database not found.");
        //Using a hash to determine uniqueness in the database, but otherwise shoving the entire
        //struct in as a JSON object.
        //I tried messed around with blobs, but it was a huge hassle dealing with that and serde
        //Originally had each item field as a column, but I was converting things just to convert
        //them back. (Which I'm still doing, but this is more straightforward).
        // I *should* implement it that way for the sake of updating menus and general database
        // nonsense, but that's not a current priority.
        connection.execute(
            "CREATE TABLE menu_db (
                id  INTEGER PRIMARY KEY,
                item_data   TEXT
            )",
            (),
        )?;
    }

    Ok(())
}

//Takes a file path to a json file and adds it to the database.
pub(crate) fn add_json_to_db(path: &str, file: &str) -> Result<(), Box<dyn Error>> {
    let items = read_from_json(file);
    let connection = Connection::open(path)?;
    //Ostensibly it checks for conflict first, so this minimizes operations on existing entries.
    //However, rather than ignore it may make sense to update. Left as is for current convenience.
    let mut statement = connection.prepare("INSERT INTO menu_db (id, item_data) \
    VALUES (?1, ?2) ON CONFLICT(id) DO NOTHING")?;

    for item in items {
        //Casting the hash to i64 because sqlite can't handle u64.
        //...that took entirely too long to figure out what was failing here.
        statement.execute(params![item.get_hash() as i64, serde_json::to_value(item).unwrap()])?;
    }

    Ok(())
}

//Fills a vec with Items from a Json file.
fn read_from_json(file: &str) -> Vec<Item> {
    let raw = fs::read_to_string(file).expect("Passed JSON file should exist, double check path.");
    let json: Vec<Item> = serde_json::from_str(&raw).unwrap();
    json
}

//Returns a vec of every item in the database.
fn db_to_vec(path: &str) -> Result<Vec<Item>, Box<dyn Error>> {
    let connection = Connection::open(path)?;
    let mut items: Vec<Item> = Vec::new();
    let mut statement = connection.prepare("SELECT item_data FROM menu_db").unwrap();

    let mut rows = statement.query([]).unwrap();

    //Kinda ugly, try using a query_map?
    while let Some(row) = rows.next().unwrap() {
        let val: serde_json::Value = row.get(0).unwrap();
        items.push(serde_json::from_value(val).unwrap());
    }
    Ok(items)
}

//Takes a vector of items (generally taken from the database, via db_to_vec) and creates a map of 
//words found in the various menus/items, and which items have that word.
fn make_map(items: Vec<Item>) -> HashMap<String, HashSet<Arc<Item>>> {
    let mut map: HashMap<String, HashSet<Arc<Item>>> = HashMap::new();
    for item in &items {
        //First time using explicit reference counting, one of those little things that took longer
        //to figure out than is suggested by how little code there is.
        //Could probably be a Weak reference? Something to learn later.
        let item_copy: Arc<Item> = Arc::new(item.clone());

        let mut all_words: Vec<String> = item.item_name.split(char::is_whitespace).map(|s| s.to_string()).collect();

        for elem in &item.ingredients {
            all_words.append(&mut elem.split(char::is_whitespace).map(|s| s.to_string()).collect::<Vec<String>>());
        }

        for word in all_words.iter_mut() {
            //Should numbers be retained?
            word.retain(|c| c.is_alphanumeric());
            word.make_ascii_lowercase();
        }

        for word in all_words {
            match map.get_mut(&word) {
                Some(x) => {
                    x.insert(Arc::clone(&item_copy));
                }
                None => {
                    map.insert(word, HashSet::from([Arc::clone(&item_copy)]));
                }
            }
        }
    }
    map
}

pub fn get_map(path: &str) -> HashMap<String, HashSet<Arc<Item>>> {
    make_map(db_to_vec(path).unwrap())
}

#[cfg(test)]
mod tests {
    use crate::menu::{add_json_to_db, db_to_vec, ensure_db, make_map};

    #[test]
    fn test_db_setup() {
        let path = "./test_db.sqlite";
        ensure_db(path).expect("Failed");
    }

    #[test]
    fn test_add_json() {
        let path = "./test_db.sqlite";
        ensure_db(path).expect("Failed");

        add_json_to_db(path, "res/bateau_04-11.json").unwrap();
        add_json_to_db(path, "res/canlis_06-03.json").unwrap();
        add_json_to_db(path, "res/lark_06-03.json").unwrap();
        add_json_to_db(path, "res/westward_05-16.json").unwrap();
    }

    #[test]
    fn test_read_db() {
        let path = "./test_db.sqlite";
        ensure_db(path).expect("Failed");

        let items = match db_to_vec(path) {
            Ok(x) => { x }
            Err(_) => todo!(),
        };

        for item in items {
            println!("{}: {}", item.item_name, item.restaurant)
        }
    }

    #[test]
    fn test_mapper() {
        let path = "./test_db.sqlite";
        let map = make_map(db_to_vec(path).unwrap());

        match map.get("oyster") {
            Some(x) => {
                for elem in x {
                    println!("{}: {:?}", elem.item_name, elem.ingredients)
                }
            }
            None => { println!("Oyster not found!") }
        }
    }
}