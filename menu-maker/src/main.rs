use std::collections::VecDeque;
use std::fs;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use poppler::PopplerDocument;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thirtyfour::{By, DesiredCapabilities, WebDriver};

//Potential struct for a restaurant
//A lot of uncertainly around how best to do this.
struct _Restaurant {
    name: String,
    menus: Vec<_Menu>,
    menu_locations: Vec<String>,
    open_status: bool,
    website: String,
}

//Potential struct for a menu.
//A lot of uncertainly around how best to do this.
struct _Menu {
    item_names: Vec<Item>,
    restaurant_name: String,
    updated: NaiveDate,
}

//Potential enum for prices.
//Not as straightforward a concept as I originally considered.
enum _Price {
    Cost(u8),
    Market(String),
    Other(String),
}

//Struct for an individual entry on a menu.
//I think "Item" is a terrible name, but it's the only word I can think of people using.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Item {
    item_name: String,
    ingredients: Vec<String>,
    updated: String,    //This should be a date, but that complicates a lot of things.
    price: String,      //This should probably be an enum, but I'm not sure how to divide options.
    restaurant: String, //This would probably benefit from being a struct.
}

//Parses a pdf menu from Bateau.
//Fortunately, everything is per-line to begin with and there are no columns, so it's fairly basic.
fn bateau_parse(mut menu: String) -> Vec<Item> {
    let naive_date = DateTime::date_naive(&Utc::now());
    let mut items: Vec<Item> = Vec::new();
    let mut prev = char::default();
    let re = Regex::new(r"(?<price>[0-9]+$)").unwrap();

    menu.retain(|c| c != '*');

    for item in menu
        .split(|c: char| {
            let result = c == '\n' || (c.is_whitespace() && prev.is_ascii_digit());
            prev = c;
            result
        })
        .map(|c| c.trim())
        .filter(|c| c.len() > 3)
    {
        //Exclude some menu lines that aren't items.
        if !item.contains(|c: char| c.is_lowercase())
            || !item.contains(|c: char| c.is_ascii_digit())
            || item.contains("minutes")
            || item.contains("chalkboard") {
            continue;
        }

        let split = item.find(|c: char| c.is_ascii_digit() || !(c.is_alphanumeric() || c.is_whitespace() || c == '&')).unwrap();
        let (name, details) = item.split_at(split);
        let temp = details.replace(" and ", ", ");

        let mut ingredients: Vec<_> = temp
            .split(',')
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|s| !s.is_empty())
            .collect();

        let last = ingredients.pop().unwrap();
        let mat = re.find(last);
        let cost: u16 = match mat {
            None => {
                ingredients.push(last);
                0
            }
            Some(val) => {
                ingredients.push(last.get(0..val.start()).unwrap().trim());
                u16::from_str(val.as_str()).unwrap()
            }
        };

        items.push(Item {
            item_name: name.to_string(),
            ingredients: ingredients.iter().map(|s| s.to_string()).collect(),
            updated: naive_date.to_string(),
            price: cost.to_string(), //Low effort bandage because I'm not sure how I want to deal with prices. 
            restaurant: "Bateau".to_string(),
        });
    }
    items
}

//Parses a pdf menu from Westward.
//Frankly, their menus are a mess. Multiple columns, multiple lines for some items, titles as names,
//inconsistent price formatting, et cetera. Attempting to parse a new menu may or may not work.
//Any better-than-temporary solution would probably require some very specific regex, possibly a
//better solution for items not being inconsistently split across columns, and hoping they don't 
//change the general layout too much.
fn west_parse(mut menu: String) -> Vec<Item> {
    let naive_date = DateTime::date_naive(&Utc::now());
    //Very ugly, each replace is O(N) and an allocation
    menu = menu.replace(" mp", " mp\n").replace("inlet, wa", "inlet, wa\n");
    let re_oyster = Regex::new(r",[[:space:]](wa)|(mp)$").unwrap();
    let re_price = Regex::new(r"[[:digit:]]+$").unwrap();

    let mut temp = String::new();
    let mut items: Vec<Item> = Vec::new();
    let mut entries: Vec<String> = Vec::new();
    for item in menu.split('\n').map(|s| s.trim()) {
        //Pretty sure each contains is O(N), so this is pretty icky.
        if !item.contains(|c: char| c.is_lowercase())
            || item.contains("consumption")
            || item.contains("parties")
            || item.contains("employees")
            || item.contains("manager")
            || item.contains("Please") {
            temp.clear();
            continue;
        }
        temp.push_str(item);
        if re_oyster.is_match(item) || re_price.is_match(item) {
            entries.push(temp.to_string());
            temp.clear();
        }
    }
    for item in entries {
        let mut temp: Vec<_> = item
            .split(|c| c == ',' || c == '/')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        //Panics if temp is empty, which could happen if things are reformated
        let ingredients: Vec<_> = temp.drain(1..).collect();

        let cost: u16 = match ingredients.last() {
            Some(val) => {
                match val.contains(|c: char| !c.is_numeric()) {
                    true => { 0 }
                    false => {
                        u16::from_str(val).unwrap()
                    }
                }
            }
            None => { 0 }
        };

        items.push(
            Item {
                item_name: temp.pop().unwrap().to_string(),
                ingredients: ingredients.iter().map(|s| s.to_string()).collect(),
                updated: naive_date.to_string(),
                price: cost.to_string(), //Low effort bandage because I'm not sure how I want to deal with prices. 
                restaurant: "Westward".to_string(),
            }
        )
    }
    items
}

//Very basic cleanup to remove consecutive spaces and newlines.
fn general_cleanup(mut menu: String) -> String {
    let mut prev = ' ';
    menu.retain(|c| {
        //Trick adapted from SO: 71864137
        let result = (c != ' ' || prev != ' ') && (c != '\n' || prev != '\n');
        // && c != '*'
        prev = c;
        result
    });
    menu
}

//Uses pdf_extract to extract the text from a pdf (shocking, I know).
//Effectively dropped because the Westward menu came out a garbled mess of characters.
//However, it actually found the columns much better than poppler does. Perhaps there's a way to
//find the columns with this, then send them poppler?
fn _extract_parse(file: &str) -> String {
    let bytes = fs::read(file).unwrap();
    let menu_string: String = pdf_extract::extract_text_from_mem(&bytes).unwrap();
    menu_string
}

//Extracts the text from a pdf. 
//This is what Okular uses, so I'm curious why this occasionally fails to notice column breaks. 
fn poppler_parse(file: &str) -> String {
    let data = PopplerDocument::new_from_file(file, None).unwrap();
    let mut menu = String::new();
    for page in data.pages() {
        menu.push_str(page.get_text().unwrap());
    }
    menu
}

fn _json_parse(file: &str) {
    let raw = fs::read_to_string(file).unwrap();
    let json: Vec<Item> = serde_json::from_str(&raw).unwrap();
    for val in json {
        println!("{}: {}", val.item_name, val.price)
    }
}

fn json_write(items: Vec<Item>, file_name: &str) {
    let mut writer = BufWriter::new(File::create(file_name).unwrap());
    serde_json::to_writer(&mut writer, &items).unwrap();
    writer.flush().unwrap();
}

//Parses the menu on the Canlis website.
//Originally used Reqwest, which worked just fine. But if I'm using a webdriver, may as well use it
//here too.
async fn canlis_parse() -> Vec<Item> {
    let naive_date = DateTime::date_naive(&Utc::now());
    let mut items: Vec<Item> = Vec::new();
    let caps = DesiredCapabilities::firefox();
    let driver = WebDriver::new("http://localhost:4444", caps).await.unwrap();

    driver.goto("https://canlis.com/menu").await.unwrap();

    //Finds each relevant section of the menu
    let elem_text = driver.find_all(By::Css("div.mb4")).await.unwrap();

    for element in elem_text {
        let item_str = element.text().await.unwrap();
        if item_str.contains("menu") {
            break;
        }
        let mut item_vec: VecDeque<_> = item_str.split('\n')
            .filter(|x| x.contains(|c: char| c.is_alphanumeric()))
            .collect();

        if item_vec.len() == 2 {
            items.push(Item {
                item_name: item_vec.pop_front().unwrap().to_string(),
                ingredients: item_vec.pop_front().unwrap().split(',').map(|x| x.trim().to_string()).collect(),
                updated: naive_date.to_string(),
                price: "n/a".to_string(), //Low effort bandage because I'm not sure how I want to deal with prices. 
                restaurant: "Canlis".to_string(),
            })
        }
    }
    // Always explicitly close the browser.
    driver.quit().await.unwrap();

    items
}

//Parses the menu on the Lark website.
//Reqwest fails (400 error, which is interesting), so I had to figure out a full webdriver.
//Breaks if geckodriver isn't running.
//Of all the menus, this was easily the one that made me the most hungry.
async fn lark_parse() -> Vec<Item> {
    let naive_date = DateTime::date_naive(&Utc::now());
    let mut items: Vec<Item> = Vec::new();
    let caps = DesiredCapabilities::firefox();
    let driver = WebDriver::new("http://localhost:4444", caps).await.unwrap();

    driver.goto("https://www.larkseattle.com/menu").await.unwrap();

    //Finds each relevant section of the menu
    let elem_text = driver.find_all(By::Css("div.sqs-html-content")).await.unwrap();

    for elem in elem_text {
        let elem_items = elem.find_all(By::Css("p")).await.unwrap();

        for item in elem_items {
            let item_str = item.text().await.unwrap();
            if item_str.contains("menu")
                || item_str.contains("party")
                || item_str.contains("Seattle") { //I feel like this one could fail if they mention a local brand
                break;
            }
            let mut item_vec: VecDeque<_> = item_str.split('\n').collect();

            if item_vec.len() == 3 {
                items.push(Item {
                    item_name: item_vec.pop_front().unwrap().to_string(),
                    ingredients: item_vec.pop_front().unwrap().split(',').map(|x| x.trim().to_string()).collect(),
                    updated: naive_date.to_string(),
                    price: item_vec.pop_front().unwrap().to_string(),
                    restaurant: "Lark".to_string(),
                })
            }
        }
    }
    // Always explicitly close the browser.
    driver.quit().await.unwrap();

    items
}

//Runs all the included parsing functions.
//Async because the webdriver functions are async, which is a little annoying.
#[tokio::main]
async fn main() {
    let mut file = "res/05.16.24-Dinner-Food.pdf";
    let mut result = general_cleanup(poppler_parse(file));
    let mut out_file = "westward_05-16.json";
    json_write(west_parse(result), out_file);
    file = "res/Bateau-a-la-carte-4.11.24.pdf";
    result = general_cleanup(poppler_parse(file));
    out_file = "bateau_04-11.json";
    json_write(bateau_parse(result), out_file);
    out_file = "canlis_06-10.json";
    json_write(canlis_parse().await, out_file);
    out_file = "lark_06-10.json";
    json_write(lark_parse().await, out_file);
}