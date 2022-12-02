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
use web_view::*;

struct BookFind {
    book_id: String,
    text: String,
}

#[derive(Debug, Clone)]
pub struct BookMetadata {
    title: String,
    author: String,
}
#[derive(Debug, Clone)]
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
    let mut fts_insert_stmt =
        fts_connection.prepare("INSERT INTO book(bookid, text) VALUES(?1, ?2)")?;

    let book_gutenberg_ids = cache.query(&json!({
        "language": "\"en\"",
        "bookshelve": "'Romantic Fiction',
                'Astounding Stories','Mystery Fiction','Erotic Fiction',
                'Mythology','Adventure','Humor','Bestsellers, American, 1895-1923',
                'Short Stories','Harvard Classics','Science Fiction','Gothic Fiction','Fantasy'",
    }))?;

    let progress_bar = ProgressBar::new(book_gutenberg_ids.len() as u64);
    progress_bar.set_style(
        ProgressStyle::with_template(
            "{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.white/blue}] ({eta})",
        )?
        .progress_chars("█  "),
    );
    progress_bar.set_message("Building full text  search db");

    for (idx, gutenberg_id) in book_gutenberg_ids.iter().enumerate() {
        progress_bar.set_position(idx as u64);

        let links = cache.get_download_links(vec![*gutenberg_id])?;

        if let Some(link) = links.first() {
            let text = get_text_from_link(&settings, link).await?;
            let stripped_text = strip_headers(text);
            let paragraphs: Split<&str> = stripped_text.split("\n\r");
            for paragraph in paragraphs {
                let paragraph_trimmed = paragraph.trim();

                if paragraph_trimmed.is_empty() {
                    continue;
                }

                if paragraph_trimmed.len() < 64 {
                    continue;
                }

                fts_insert_stmt
                    .execute((format!("${}${}$", gutenberg_id, link), paragraph_trimmed))?;
            }
        }
    }
    progress_bar.finish();
    Ok(())
}

fn get_lit_clock_data(
    db_filename: &str,
    time_now: DateTime<Local>,
) -> Result<LitClockEntry, Error> {
    let lit_clock_db = Box::new(Connection::open(db_filename)?);
    let mut m = lit_clock_db.prepare("SELECT distinct(time) as t FROM littime order by t;")?;
    let available_times: Vec<u32> = m
        .query_map((), |row| Ok(row.get::<usize, u32>(0)?))?
        .map(|x| match x {
            Ok(_x) => _x,
            Err(_) => 0,
        })
        .collect();

    let mut number_search = time_now.hour12().1 * 100 + time_now.minute();
    let find_result = available_times.iter().rfind(|&&x| x <= number_search);

    if let Some(find) = find_result {
        number_search = *find;
    }
    let mut e =
        lit_clock_db.prepare("SELECT text, author,title,link,time FROM littime WHERE time = ?1")?;
    let clock_entries_results: Vec<rusqlite::Result<LitClockEntry>> = e.query_map((number_search,), |row| {
        Ok(LitClockEntry {
            paragraph: row.get(0)?,
            author: row.get(1)?,
            title: row.get(2)?,
            link: row.get(3)?,
        })
    })?.collect();
    let pick = clock_entries_results.choose(&mut rand::thread_rng());
    if let Some(p) = pick {
        if let Ok(d) = p {
            return Ok(d.clone());
        }
    }
    return Err(Error::InvalidResult("no time".to_string()));
}

fn generate_lit_clock_db(cache: &mut SQLiteCache, db_filename: &str) -> Result<(), Error> {
    let fts_connection = Box::new(Connection::open(db_filename)?);
    if !std::path::Path::new(db_filename).exists() {
        let lit_clock_db = Box::new(Connection::open(db_filename)?);

        lit_clock_db.execute_batch(
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
        let mut insert = lit_clock_db.prepare(
            "INSERT INTO littime(time, text, author, title, link) VALUES(?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut book_metadata_map: HashMap<usize, BookMetadata> = HashMap::new();

        let progress_bar = ProgressBar::new((13 * 60) as u64);
        progress_bar.set_style(
            ProgressStyle::with_template(
                "{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.white/blue}] ({eta})",
            )?
            .progress_chars("█  "),
        );
        progress_bar.set_message(format!("Building clock db {}", db_filename));

        for hour in 1..13 {
            for minute in 0..60 {
                let time_number = hour * 100 + minute;
                progress_bar.set_position((hour * 60 + minute) as u64);

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
                        fts_connection.prepare("SELECT bookid, highlight(book, 1,'<b>', '</b>')  FROM book WHERE text MATCH ?1 ")?;
                let res_iter = stmt.query_map((&query_words,), |row| {
                    Ok(BookFind {
                        book_id: row.get(0)?,
                        text: row.get(1)?,
                    })
                })?;

                for entry in res_iter {
                    let book_paragraph = entry?;
                    let book_id;
                    let link: String;
                    scan!(book_paragraph.book_id.bytes() => "${}${}$", book_id, link);

                    let mut metadata: Option<&BookMetadata> = None;
                    if let Some(data) = book_metadata_map.get(&book_id) {
                        metadata = Some(data);
                    } else {
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
                            book_metadata_map.insert(book_id, data);
                            metadata = book_metadata_map.get(&book_id);
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
            progress_bar.finish();
        }
        lit_clock_db.execute_batch("CREATE INDEX time_idx ON littime (`time` ASC);")?;
        lit_clock_db.flush_prepared_statement_cache();
    }
    Ok(())
}

fn show_app(html_content: &str, lit_clock_db_filename: &str)-> Result<(), Error> {
    web_view::builder()
        .title("Gutenberg clock")
        .content(Content::Html(html_content))
        .size(800, 600)
        .resizable(false)
        .debug(true)
        .user_data(())
        .invoke_handler(|_webview, _arg| match _arg {
            "refreshtime" => {
                let time_now = Local::now();
                let rs = get_lit_clock_data(lit_clock_db_filename, time_now);
                if let Ok(r) = rs {
                    let time_string = format!("{}:{}", time_now.hour12().1, time_now.minute());
                    let mut paragraph = r.paragraph.replace("\"", "'");
                    paragraph = paragraph.lines().collect::<Vec<&str>>().join(" ");
                    let title = r.title.replace("\"", "'");
                    let author = r.author.replace("\"", "'");
                    let eval_func = format!(
                        "updateData(\"{}\", \"{}\", \"{}\", \"{}\", \"{}\");",
                        time_string, &paragraph, &title, &author, &r.link
                    );
                    _webview.eval(eval_func.as_str())?;
                }
                Ok(())
            }
            _ => {
                unimplemented!();
            }
        })
        .run()
        .unwrap();
    Ok(())
}

async fn exec() -> Result<(), Error> {
    let fts_filename = "fts.db";
    let lit_clock_db_filename = "lit_clock.db";

    let settings = GutenbergCacheSettings::default();

    if !std::path::Path::new(&settings.cache_filename).exists() {
        setup_sqlite(&settings, false, true).await?;
    }

    let mut cache = SQLiteCache::get_cache(&settings)?;
    
    if !std::path::Path::new(fts_filename).exists() {
        generate_fts(&mut cache, settings, fts_filename).await?;
    }

    if !std::path::Path::new(lit_clock_db_filename).exists() {
        generate_lit_clock_db(&mut cache, lit_clock_db_filename)?;
    }

    show_app(include_str!("clock.html"), lit_clock_db_filename)?;
  
    Ok(())
}

#[tokio::main]
async fn main() {
    match exec().await {
        Ok(_e) => {}
        Err(_e) => println!("program failed with error: {}", _e.to_string()),
    }
}
