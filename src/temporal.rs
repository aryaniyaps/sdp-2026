//! Reads the time expressions in a recall query without calling a model.
//!
//! The recall path used to ask a model for a date window whenever the query held a word such as "when" or
//! "before", which cost two to four seconds and for ordinary questions came back with no window at all.
//! This parser recognises a short list of expressions and turns each into the same half-open event range
//! (`from` inclusive, `to` exclusive) or `as_of` instant the model planner returned. All times are UTC and
//! relative expressions are measured from the request's reference date.
//!
//! Recognised, each matched on whole lower-case words:
//! - `yesterday`, `today`
//! - `last` or `this` followed by `week`, `month` or `year` (a week runs Monday to Sunday)
//! - `N day(s)|week(s)|month(s)|year(s) ago`, where N is a number or one to twelve (`a` and `an` are 1);
//!   the range is the day, week, month or year that contains that date
//! - a month name after `in`, `during`, `of` or `since`, with an optional four digit year; without a year it
//!   is the latest such month that has begun, so `in March` asked in October means this March
//! - a four digit year after `in`, `during` or `since`
//! - an ISO date such as `2026-03-03`: that day, or from that day after `since`
//! - `as of` and an ISO date: the state at the end of that day (`as_of`)
//!
//! Anything else gives no window, so recall runs without a time filter.
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

/// A recognised time expression: the bounds it gives and the words it matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub as_of: Option<DateTime<Utc>>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub phrase: String,
}

const MONTHS: [(&str, u32); 21] = [
    ("january", 1),
    ("jan", 1),
    ("february", 2),
    ("feb", 2),
    ("march", 3),
    ("mar", 3),
    ("april", 4),
    ("apr", 4),
    ("may", 5),
    ("june", 6),
    ("jun", 6),
    ("july", 7),
    ("jul", 7),
    ("august", 8),
    ("aug", 8),
    ("september", 9),
    ("sept", 9),
    ("sep", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
];
const NUMBER_WORDS: [(&str, i64); 14] = [
    ("a", 1),
    ("an", 1),
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
];

fn day_start(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight exists"))
}
fn month_start(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, 1).expect("valid month")
}
fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}
/// The year and month `back` months before `year`-`month`.
fn months_back(year: i32, month: u32, back: i64) -> (i32, u32) {
    let total = i64::from(year) * 12 + i64::from(month) - 1 - back;
    (total.div_euclid(12) as i32, total.rem_euclid(12) as u32 + 1)
}
fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}
fn day_window(date: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    (day_start(date), day_start(date + Duration::days(1)))
}
fn week_window(date: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let monday = week_start(date);
    (day_start(monday), day_start(monday + Duration::days(7)))
}
fn month_window(year: i32, month: u32) -> (DateTime<Utc>, DateTime<Utc>) {
    let (ny, nm) = next_month(year, month);
    (
        day_start(month_start(year, month)),
        day_start(month_start(ny, nm)),
    )
}
fn year_window(year: i32) -> (DateTime<Utc>, DateTime<Utc>) {
    (
        day_start(month_start(year, 1)),
        day_start(month_start(year + 1, 1)),
    )
}
/// The window that is `back` units before the reference date, for `day`, `week`, `month` or `year`.
fn unit_window(unit: &str, today: NaiveDate, back: i64) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    Some(match unit.trim_end_matches('s') {
        "day" => day_window(today - Duration::days(back)),
        "week" => week_window(today - Duration::days(7 * back)),
        "month" => {
            let (y, m) = months_back(today.year(), today.month(), back);
            month_window(y, m)
        }
        "year" => year_window(today.year() - i32::try_from(back).ok()?),
        _ => return None,
    })
}
fn month_number(word: &str) -> Option<u32> {
    MONTHS
        .iter()
        .find(|(name, _)| *name == word)
        .map(|(_, n)| *n)
}
fn number(word: &str) -> Option<i64> {
    word.parse::<i64>()
        .ok()
        .filter(|n| (0..=1000).contains(n))
        .or_else(|| {
            NUMBER_WORDS
                .iter()
                .find(|(w, _)| *w == word)
                .map(|(_, n)| *n)
        })
}
fn year_number(word: &str) -> Option<i32> {
    (word.len() == 4)
        .then(|| word.parse::<i32>().ok())
        .flatten()
        .filter(|y| (1900..=2100).contains(y))
}
fn iso_date(word: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(word, "%Y-%m-%d").ok()
}
fn is_unit(word: &str) -> bool {
    matches!(
        word,
        "day" | "days" | "week" | "weeks" | "month" | "months" | "year" | "years"
    )
}

/// Finds the first time expression in the query, or `None` when there is none.
pub fn plan(query: &str, reference: DateTime<Utc>) -> Option<Window> {
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .map(|w| w.trim_matches('-').to_string())
        .filter(|w| !w.is_empty())
        .collect();
    let today = reference.date_naive();
    let word = |i: usize| words.get(i).map(String::as_str);
    let window = |range: (DateTime<Utc>, DateTime<Utc>), n: usize, at: usize| Window {
        as_of: None,
        from: Some(range.0),
        to: Some(range.1),
        phrase: words[at..at + n].join(" "),
    };
    for i in 0..words.len() {
        let w = words[i].as_str();
        match w {
            "yesterday" => return Some(window(day_window(today - Duration::days(1)), 1, i)),
            "today" => return Some(window(day_window(today), 1, i)),
            "last" | "this" => {
                if let Some(unit) = word(i + 1).filter(|u| matches!(*u, "week" | "month" | "year"))
                {
                    let back = i64::from(w == "last");
                    return unit_window(unit, today, back).map(|r| window(r, 2, i));
                }
            }
            "as" if word(i + 1) == Some("of") => {
                if let Some(date) = word(i + 2).and_then(iso_date) {
                    let end = day_start(date + Duration::days(1)) - Duration::seconds(1);
                    return Some(Window {
                        as_of: Some(end),
                        from: None,
                        to: None,
                        phrase: words[i..i + 3].join(" "),
                    });
                }
            }
            "in" | "during" | "of" | "since" => {
                let since = w == "since";
                if let Some(month) = word(i + 1).and_then(month_number) {
                    let year = word(i + 2).and_then(year_number);
                    let n = if year.is_some() { 3 } else { 2 };
                    let year = year.unwrap_or(if month > today.month() {
                        today.year() - 1
                    } else {
                        today.year()
                    });
                    let (from, to) = month_window(year, month);
                    return Some(if since {
                        Window {
                            as_of: None,
                            from: Some(from),
                            to: None,
                            phrase: words[i..i + n].join(" "),
                        }
                    } else {
                        window((from, to), n, i)
                    });
                }
                if w != "of"
                    && let Some(year) = word(i + 1).and_then(year_number)
                {
                    let (from, to) = year_window(year);
                    return Some(if since {
                        Window {
                            as_of: None,
                            from: Some(from),
                            to: None,
                            phrase: words[i..i + 2].join(" "),
                        }
                    } else {
                        window((from, to), 2, i)
                    });
                }
                if since && let Some(date) = word(i + 1).and_then(iso_date) {
                    return Some(Window {
                        as_of: None,
                        from: Some(day_start(date)),
                        to: None,
                        phrase: words[i..i + 2].join(" "),
                    });
                }
            }
            _ => {}
        }
        if let Some(date) = iso_date(w) {
            return Some(window(day_window(date), 1, i));
        }
        if let Some(n) = number(w)
            && word(i + 2) == Some("ago")
            && word(i + 1).is_some_and(is_unit)
        {
            return unit_window(word(i + 1).unwrap_or_default(), today, n).map(|r| window(r, 3, i));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Tuesday, 6 October 2026.
    fn reference() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
    }
    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }
    fn range(query: &str) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
        let w = plan(query, reference()).unwrap_or_else(|| panic!("no window for {query:?}"));
        assert_eq!(w.as_of, None, "{query:?}");
        (w.from, w.to)
    }
    #[test]
    fn relative_days_weeks_months_and_years() {
        assert_eq!(
            range("what did I decide yesterday"),
            (Some(at(2026, 10, 5)), Some(at(2026, 10, 6)))
        );
        assert_eq!(
            range("anything from today?"),
            (Some(at(2026, 10, 6)), Some(at(2026, 10, 7)))
        );
        assert_eq!(
            range("what changed last week"),
            (Some(at(2026, 9, 28)), Some(at(2026, 10, 5)))
        );
        assert_eq!(
            range("this week"),
            (Some(at(2026, 10, 5)), Some(at(2026, 10, 12)))
        );
        assert_eq!(
            range("last month"),
            (Some(at(2026, 9, 1)), Some(at(2026, 10, 1)))
        );
        assert_eq!(
            range("this month"),
            (Some(at(2026, 10, 1)), Some(at(2026, 11, 1)))
        );
        assert_eq!(
            range("last year"),
            (Some(at(2025, 1, 1)), Some(at(2026, 1, 1)))
        );
    }
    #[test]
    fn counted_units_ago_use_the_period_that_holds_that_date() {
        assert_eq!(
            range("3 days ago"),
            (Some(at(2026, 10, 3)), Some(at(2026, 10, 4)))
        );
        assert_eq!(
            range("two weeks ago"),
            (Some(at(2026, 9, 21)), Some(at(2026, 9, 28)))
        );
        assert_eq!(
            range("a month ago"),
            (Some(at(2026, 9, 1)), Some(at(2026, 10, 1)))
        );
        assert_eq!(
            range("5 months ago"),
            (Some(at(2026, 5, 1)), Some(at(2026, 6, 1)))
        );
        assert_eq!(
            range("10 months ago"),
            (Some(at(2025, 12, 1)), Some(at(2026, 1, 1)))
        );
        assert_eq!(
            range("2 years ago"),
            (Some(at(2024, 1, 1)), Some(at(2025, 1, 1)))
        );
    }
    #[test]
    fn months_years_and_dates() {
        assert_eq!(
            range("what happened in March 2025"),
            (Some(at(2025, 3, 1)), Some(at(2025, 4, 1)))
        );
        assert_eq!(
            range("what happened in march"),
            (Some(at(2026, 3, 1)), Some(at(2026, 4, 1)))
        );
        assert_eq!(
            range("during December"),
            (Some(at(2025, 12, 1)), Some(at(2026, 1, 1)))
        );
        assert_eq!(
            range("in october"),
            (Some(at(2026, 10, 1)), Some(at(2026, 11, 1)))
        );
        assert_eq!(
            range("in 2024"),
            (Some(at(2024, 1, 1)), Some(at(2025, 1, 1)))
        );
        assert_eq!(
            range("on 2026-03-03"),
            (Some(at(2026, 3, 3)), Some(at(2026, 3, 4)))
        );
        assert_eq!(range("since march 2025"), (Some(at(2025, 3, 1)), None));
        assert_eq!(range("since 2025"), (Some(at(2025, 1, 1)), None));
        assert_eq!(range("since 2026-09-30"), (Some(at(2026, 9, 30)), None));
    }
    #[test]
    fn as_of_gives_the_state_at_the_end_of_that_day() {
        let w = plan("what was true as of 2026-03-03", reference()).unwrap();
        assert_eq!(
            w.as_of,
            Some(Utc.with_ymd_and_hms(2026, 3, 3, 23, 59, 59).unwrap())
        );
        assert_eq!((w.from, w.to), (None, None));
        assert_eq!(w.phrase, "as of 2026-03-03");
    }
    #[test]
    fn the_matched_words_are_reported() {
        assert_eq!(
            plan("What did we decide LAST week?", reference())
                .unwrap()
                .phrase,
            "last week"
        );
        assert_eq!(
            plan("it happened three days ago", reference())
                .unwrap()
                .phrase,
            "three days ago"
        );
    }
    #[test]
    fn ordinary_questions_get_no_window() {
        for q in [
            "what happens when two sources disagree",
            "what does the Pi extension do before a prompt",
            "how does the spool keep text when the service is down",
            "what happens after extraction finishes",
            "I may be late",
            "what is my last name",
            "how long ago was it",
            "may I ask a question",
            "this is a test",
            "in the beginning",
            "version 2024 of the schema",
            "how many days are in a week",
        ] {
            assert_eq!(plan(q, reference()), None, "{q:?}");
        }
    }
}
