use chrono::prelude::*;
use core::str::Split;
use gutenberg_rs::error::Error;
use gutenberg_rs::settings::GutenbergCacheSettings;
use gutenberg_rs::setup_sqlite;
use gutenberg_rs::sqlite_cache::SQLiteCache;
use gutenberg_rs::text_get::{get_text_from_link, strip_headers};
use indicatif::{ProgressBar, ProgressStyle};
use rand::seq::SliceRandom;
use rusqlite::Connection;
use serde_json::json;
use std::collections::HashMap;
use utils::all_formats_to_text;
mod utils;
use text_io::scan;

pub struct BookFind {
    bookid: String,
    text: String,
}

#[derive(Debug, Clone)]
pub struct BookMetadata {
    title: String,
    author: String,
}
pub struct LitClockEntry {
    paragraph: String,
    author: String,
    title: String,
    link: String,
}

async fn generate_fts(
    cache: &mut SQLiteCache,
    settings: GutenbergCacheSettings,
    fts_filename: &str,
) -> Result<(), Error> {
    let fts_connection = Box::new(Connection::open(fts_filename)?);
    fts_connection.execute("CREATE VIRTUAL TABLE book USING fts5(bookid, text);", ())?;
    fts_connection.execute_batch("PRAGMA journal_mode = OFF;PRAGMA synchronous = 0;PRAGMA cache_size = 1000000;PRAGMA locking_mode = EXCLUSIVE;PRAGMA temp_store = MEMORY;")?;
    let mut fts_stmt = fts_connection.prepare("INSERT INTO book(bookid, text) VALUES(?1, ?2)")?;

    let books = cache.query(&json!({
        "language": "\"en\"",
        "bookshelve": "'Romantic Fiction',
                'Astounding Stories','Mystery Fiction','Erotic Fiction',
                'Mythology','Adventure','Humor','Bestsellers, American, 1895-1923',
                'Short Stories','Harvard Classics','Science Fiction','Gothic Fiction','Fantasy'",
    }))?;

    let pb = ProgressBar::new(books.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.white/blue}] ({eta})",
        )?
        .progress_chars("█  "),
    );
    pb.set_message("Building full text  search db");

    for (idx, gutenberg_id) in books.iter().enumerate() {
        pb.set_position(idx as u64);
        let links = cache.get_download_links(vec![*gutenberg_id])?;
        match links.first() {
            Some(link) => {
                let text = get_text_from_link(&settings, link).await?;
                let stripped_text = strip_headers(text);
                let paragraphs: Split<&str> = stripped_text.split("\n\r");
                for paragraph in paragraphs {
                    if paragraph.trim().is_empty() {
                        continue;
                    }

                    fts_stmt.execute((format!("${}${}$", gutenberg_id, link), paragraph))?;
                }
            }
            None => {}
        };
    }
    pb.finish();
    Ok(())
}

async fn exec() -> Result<(), Error> {
    // let's do something fun in this example :
    // - create the cache
    // - download some english books from particular shelves
    // - search for a certain time mention in all books
    // - display the paragraph with the time mention

    // here we create the cache settings with the default values
    let settings = GutenbergCacheSettings::default();

    // generate the sqlite cache (this will download, parse and create the db)
    if !std::path::Path::new(&settings.cache_filename).exists() {
        setup_sqlite(&settings, false, true).await?;
    }
    let fts_filename = "fts.db";
    let final_filename = "lit_clock.db";
    
    // we grab the newly create cache
    let mut cache = SQLiteCache::get_cache(&settings)?;
    if !std::path::Path::new(fts_filename).exists() {
        generate_fts(&mut cache, settings, fts_filename).await?;
    }
    let fts_connection = Box::new(Connection::open(fts_filename)?);
    if !std::path::Path::new(final_filename).exists() {
        let cfinal_table = Box::new(Connection::open(final_filename)?);

        cfinal_table.execute_batch(
            "CREATE TABLE littime (
            id	INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE,
            time INTEGER,
            text TEXT,
            author TEXT,
            title TEXT,
            link TEXT
        );
        PRAGMA journal_mode = OFF;PRAGMA synchronous = 0;PRAGMA cache_size = 1000000;PRAGMA locking_mode = EXCLUSIVE;PRAGMA temp_store = MEMORY;"
        )?;
        let mut insert = cfinal_table.prepare(
            "INSERT INTO littime(time, text, author, title, link) VALUES(?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut book_metadata: HashMap<usize, BookMetadata> = HashMap::new();

        let pb = ProgressBar::new((13 * 60) as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.white/blue}] ({eta})",
            )?
            .progress_chars("█  "),
        );
        pb.set_message(format!("Building clock db {}", final_filename));
    
        for hour in 1..13 {
            for minute in 0..60 {
                let time_number = hour * 100 + minute;
                pb.set_position((hour * 60 + minute) as u64);

                let word_times = all_formats_to_text(hour, minute)?;
                let mut query_words = "".to_string();
                for (idx, time_variant) in word_times.iter().enumerate() {
                    let mut word_time = time_variant.replace("'", " ");
                    word_time = format!("\"{}\"", word_time);
                    query_words = match idx {
                        0 => format!("{}", word_time),
                        _ => format!("{} OR {}", query_words, word_time),
                    };
                }
                let mut stmt =
                        fts_connection.prepare("SELECT bookid, snippet(book, 1,'<b>', '</b>', '', 64)  FROM book WHERE text MATCH ?1 ")?;
                let res_iter = stmt.query_map((&query_words,), |row| {
                    Ok(BookFind {
                        bookid: row.get(0)?,
                        text: row.get(1)?,
                    })
                })?;

                for entry in res_iter {
                    let book_paragraph = entry?;
                    let book_id;
                    let link: String;
                    scan!(book_paragraph.bookid.bytes() => "${}${}$", book_id, link);

                    let mut metadata: Option<&BookMetadata> = None;
                    if let Some(data) = book_metadata.get(&book_id) {
                        metadata = Some(data);
                    }
                    else {
                         let query = "SELECT 
                         titles.name, 
                         authors.name 
                         FROM titles, books, authors, book_authors 
                         WHERE books.id = book_authors.bookid 
                         AND authors.id = book_authors.authorid 
                         AND titles.bookid = books.id 
                         AND books.gutenbergbookid = ?1";

                        let res_meta = cache.connection.query_row(query, (book_id,), |row| {
                            Ok(BookMetadata {
                                title: row.get(0)?,
                                author: row.get(1)?,
                            })
                        });
                        
                        if let Ok(data) = res_meta {
                            book_metadata.insert(book_id, data);
                            metadata = book_metadata.get(&book_id);
                        }
                    }
                   
                    if let Some(data) = metadata {
                        insert.execute((
                            time_number,
                            book_paragraph.text,
                            &data.author,
                            &data.title,
                            link,
                        ))?;
                    }
                }
            }
            pb.finish();
        }
        cfinal_table.execute_batch("CREATE INDEX time_idx ON littime (`time` ASC);")?;
        cfinal_table.flush_prepared_statement_cache();
    }

    let cfinal_table = Box::new(Connection::open(final_filename)?);
    let mut m = cfinal_table.prepare("SELECT distinct(time) as t FROM littime order by t;")?;

    let available_times = m.query_map((), |row| Ok(row.get::<usize, u32>(0)?))?;

    let time_now = Local::now();
    let mut number_search = time_now.hour12().1 * 100 + time_now.minute();
    for time in available_times {
        let time_value = time?;
        if number_search <= time_value {
            number_search = time_value;
            break;
        }
    }
    let mut e =
        cfinal_table.prepare("SELECT text, author,title,link,time FROM littime WHERE time = ?1")?;
    let fres = e.query_map((number_search,), |row| {
        Ok(LitClockEntry {
            paragraph: row.get(0)?,
            author: row.get(1)?,
            title: row.get(2)?,
            link: row.get(3)?,
        })
    })?;
    let collect_a: Vec<rusqlite::Result<LitClockEntry>> = fres.collect();
    let picked = collect_a.choose(&mut rand::thread_rng());
    match picked {
        Some(pick_result) => match pick_result {
            Ok(r) => {
                println!("{}", r.paragraph);
                println!("--{}  by {}", r.title, r.author);
                println!("{}", r.link);
                println!("------------------------------------------------------------------");
            }
            Err(_) => {}
        },
        None => {}
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    match exec().await {
        Ok(_e) => {}
        Err(_e) => println!("program failed with error: {}", _e.to_string()),
    }
}
