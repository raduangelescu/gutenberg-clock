use gutenberg_rs::error::Error;

// this is a helper function that converts a time (hours:minutes) into spoken english time
fn time_to_text(hour: usize, minute: usize) -> Result<String, Error> {
    let nums = vec![
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
        "twenty one",
        "twenty two",
        "twenty three",
        "twenty four",
        "twenty five",
        "twenty six",
        "twenty seven",
        "twenty eight",
        "twenty nine",
    ];
    match minute {
        0 => Ok(format!("{} o'clock", nums[hour])),
        1 => Ok(format!("one minute past {}", nums[hour])),
        59 => Ok(format!("one minute to {}", nums[hour])),
        15 => Ok(format!("quarter past {}", nums[hour])),
        30 => Ok(format!("half past {}", nums[hour])),
        45 => Ok(format!("quarter to {}", nums[hour])),
        _ => {
            if minute <= 30 {
                Ok(format!("{} minutes past {}", nums[minute], nums[hour]))
            } else if minute > 30 {
                Ok(format!(
                    "{} minutes to {}",
                    nums[60 - minute],
                    nums[(hour % 12) + 1]
                ))
            } else {
                Err(Error::InvalidResult(String::from("bad time")))
            }
        }
    }
}

pub fn all_formats_to_text(hour: usize, minute: usize) -> Result<Vec<String>, Error> {
    let mut times = vec![];
    times.push(time_to_text(hour, minute)?);
    //times.push(format!("{}:{}", hour, minute));
    //times.push(format!("{}.{}", hour, minute));
    Ok(times)
}
