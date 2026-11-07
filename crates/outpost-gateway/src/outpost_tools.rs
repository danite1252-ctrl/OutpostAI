//! Outpost's own tools in chat: making a routine from what the person says ("remind me at 5
//! to call Mum", "every morning, check the Paper downloads page"), and listing their routines.
//!
//! Offered to owners and members in chats they're in (not in a routine's own runs: nobody is
//! there to say yes). A routine is only made once the person says yes to it.

use outpost_core::routines::{self, Notify, RoutineInput, Schedule};
use serde_json::{json, Value};

use crate::pc_tools::{PcTool, PcToolServer};
use crate::AppState;

/// The id of Outpost's own tool server (never a stored tool's id: those are UUIDs).
pub const SERVER_ID: &str = "outpost";

/// What these tools work for: the person, their chat's persona, and their clock.
#[derive(Clone)]
pub struct Context {
    pub state: AppState,
    pub account_id: String,
    pub persona_id: Option<String>,
    pub time_zone: Option<String>,
    pub utc_offset_minutes: Option<i32>,
    /// What the person wrote (a time they gave as "in 5 minutes" is taken from it, not from
    /// the model's sums: smaller models get those wrong).
    pub asked: String,
}

pub fn server() -> PcToolServer {
    PcToolServer {
        id: SERVER_ID.into(),
        name: "Outpost".into(),
        tools: vec![
            PcTool {
                name: "make_routine".into(),
                description: "Set up a routine: something you (this persona) will do on your own later or regularly, \
                    with the user told the result (a notification). Use it when the user asks to be reminded of \
                    something, or for something to happen later or on a schedule (\"remind me at 5 to call Mum\", \
                    \"every weekday at 7 give me the weather\", \"check that page every 6 hours\"). The user is asked to \
                    OK it first. Don't use it for things to do now."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "A short name, e.g. \"Call Mum\" or \"Morning brief\"" },
                        "instructions": { "type": "string", "description": "What to do each time it runs, as instructions to yourself, e.g. \"Remind the user to call Mum.\" or \"Check https://papermc.io/downloads/paper for the newest build and say if it's newer than last time.\"" },
                        "when": { "type": "string", "enum": ["once", "daily", "weekdays", "days", "every_hours"], "description": "once: one time; daily, weekdays or days (chosen days) at a time; every_hours: every few hours" },
                        "in_minutes": { "type": "integer", "description": "For once, when the user says \"in ...\": minutes from now (in 5 minutes = 5, in 2 hours = 120)" },
                        "at": { "type": "string", "description": "For once: the user's local date and time, \"YYYY-MM-DD HH:MM\" (24-hour)" },
                        "time": { "type": "string", "description": "For daily, weekdays or days: the time, \"HH:MM\" (24-hour), e.g. \"07:30\"" },
                        "days": { "type": "array", "items": { "type": "string", "enum": ["sun", "mon", "tue", "wed", "thu", "fri", "sat"] }, "description": "For days: which days" },
                        "hours": { "type": "integer", "description": "For every_hours: how many hours apart (1 to 168)" },
                        "notify": { "type": "string", "enum": ["always", "worth"], "description": "always (default): tell the user every time; worth: only when there's something new (for checks)" }
                    },
                    "required": ["name", "instructions", "when"]
                }),
                read_only: Some(false),
            },
            PcTool {
                name: "list_routines".into(),
                description: "The user's routines: what each does, when it runs next, and how its last run went.".into(),
                input_schema: json!({ "type": "object", "properties": {} }),
                read_only: Some(true),
            },
        ],
        withheld: Vec::new(),
        withheld_by: None,
        ask_first: Vec::new(),
    }
}

/// The system prompt's note about them.
pub const NOTE: &str = "\n\nOutpost: when the user asks to be reminded, or for something to happen later or regularly, \
    use make_routine (the user OKs it). Say what it will do and when, in their words. To change, pause or delete a \
    routine, tell them to use Routines (on the PC: in the sidebar; on the phone: You › Routines).";

/// A routine the model asked for, checked, with how it's described to the person.
#[derive(Debug)]
pub struct Wanted {
    pub input: RoutineInput,
    /// "Call Mum, once, Sat 7 Nov 17:00"
    pub summary: String,
}

fn zone(ctx: &Context) -> jiff::tz::TimeZone {
    routines::zone(ctx.time_zone.as_deref(), ctx.utc_offset_minutes)
}

/// Read make_routine's arguments: what it would make, or what's wrong (said to the model).
pub fn wanted(ctx: &Context, args: &Value) -> Result<Wanted, String> {
    parse(ctx.time_zone.clone(), ctx.utc_offset_minutes, ctx.persona_id.clone(), args, &ctx.asked, outpost_core::now())
}

/// "in 5 minutes", "in an hour", "in half an hour", "10 mins from now": how many minutes from
/// now the person said, when they said exactly one such thing.
fn said_in_minutes(text: &str) -> Option<i64> {
    let lower = text.to_lowercase().replace(['.', ',', '!', '?'], " ");
    let words: Vec<&str> = lower.split_whitespace().collect();
    let number = |w: &str| -> Option<f64> {
        Some(match w {
            "a" | "an" | "one" => 1.0,
            "two" | "couple" => 2.0,
            "three" => 3.0,
            "four" => 4.0,
            "five" => 5.0,
            "ten" => 10.0,
            "fifteen" => 15.0,
            "twenty" => 20.0,
            "thirty" => 30.0,
            "half" => 0.5,
            _ => w.parse().ok()?,
        })
    };
    let unit = |w: &str| -> Option<f64> {
        let w = w.trim_end_matches('s');
        match w {
            "min" | "minute" | "mins" => Some(1.0),
            "hour" | "hr" => Some(60.0),
            _ if w.ends_with("min") => Some(1.0),
            _ => None,
        }
    };
    let mut found: Vec<i64> = Vec::new();
    for i in 0..words.len() {
        // "<n> <unit>", "<n><unit>" (5min), "half an hour", "a couple of minutes"
        let (n, rest) = match number(words[i]) {
            Some(n) => (n, i + 1),
            None => {
                let digits: String = words[i].chars().take_while(char::is_ascii_digit).collect();
                match (digits.parse::<f64>().ok(), unit(&words[i][digits.len()..])) {
                    (Some(n), Some(u)) if !digits.is_empty() => {
                        let before = i.checked_sub(1).map(|j| words[j]);
                        if before == Some("in") || words.get(i + 1..i + 3).is_some_and(|w| w == ["from", "now"]) {
                            found.push((n * u).round() as i64);
                        }
                        continue;
                    }
                    _ => continue,
                }
            }
        };
        let mut j = rest;
        while words.get(j).is_some_and(|w| ["a", "an", "of"].contains(w)) {
            j += 1;
        }
        let Some(u) = words.get(j).and_then(|w| unit(w)) else { continue };
        let before = i.checked_sub(1).map(|k| words[k]);
        let after_now = words.get(j + 1..j + 3).is_some_and(|w| w == ["from", "now"]);
        if before == Some("in") || after_now {
            found.push((n * u).round() as i64);
        }
    }
    found.dedup();
    match found.as_slice() {
        [m] if *m > 0 => Some(*m),
        _ => None,
    }
}

/// "in 1 minute", "in 2 hours 5 minutes": how far off a time is, for the person to check.
fn from_now(secs: i64) -> String {
    let mins = (secs + 30) / 60;
    let plural = |n: i64, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
    match (mins / 60, mins % 60) {
        (0, m) => format!("in {}", plural(m.max(1), "minute")),
        (h, 0) => format!("in {}", plural(h, "hour")),
        (h, m) => format!("in {} {}", plural(h, "hour"), plural(m, "minute")),
    }
}

fn parse(
    time_zone: Option<String>,
    utc_offset_minutes: Option<i32>,
    persona_id: Option<String>,
    args: &Value,
    asked: &str,
    now: i64,
) -> Result<Wanted, String> {
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let name = text("name").ok_or("Give the routine a name.")?;
    let instructions = text("instructions").ok_or("Say what the routine should do.")?;
    let tz = routines::zone(time_zone.as_deref(), utc_offset_minutes);
    let time = || -> Result<String, String> {
        let t = text("time").ok_or("Give the time, like \"07:30\".")?;
        let (h, m) = t.split_once(':').ok_or("Give the time like \"07:30\" (24-hour).")?;
        let (h, m): (u8, u8) = (h.trim().parse().map_err(|_| "Give the time like \"07:30\".")?, m.trim().parse().map_err(|_| "Give the time like \"07:30\".")?);
        if h > 23 || m > 59 {
            return Err("Give the time like \"07:30\" (24-hour).".into());
        }
        Ok(format!("{h:02}:{m:02}"))
    };
    let schedule = match text("when").unwrap_or("once") {
        "once" => {
            let at = match (said_in_minutes(asked), args.get("in_minutes").and_then(Value::as_i64), text("at")) {
                // The person said "in 5 minutes": that, whatever sums the model did.
                (Some(m), _, _) => now + m * 60,
                (None, Some(m), _) if m > 0 => now + m * 60,
                (None, _, Some(at)) => routines::local_time(&tz, at).ok_or("Give \"at\" like \"2026-11-07 17:00\" (the user's local time).")?,
                _ => return Err("For once, give in_minutes or at.".into()),
            };
            if at <= now {
                return Err("That time has passed: give a time from now on.".into());
            }
            Schedule::Once { at }
        }
        "daily" => Schedule::Days { time: time()?, days: (0..7).collect() },
        "weekdays" => Schedule::Days { time: time()?, days: vec![1, 2, 3, 4, 5] },
        "days" => {
            const NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
            let days: Vec<u8> = args
                .get("days")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter_map(|d| NAMES.iter().position(|n| d.trim().to_lowercase().starts_with(n)))
                        .map(|i| i as u8)
                        .collect()
                })
                .unwrap_or_default();
            if days.is_empty() {
                return Err("For days, give which days, like [\"mon\", \"thu\"].".into());
            }
            Schedule::Days { time: time()?, days }
        }
        "every_hours" => {
            let hours = args.get("hours").and_then(Value::as_u64).unwrap_or(0);
            if !(1..=168).contains(&hours) {
                return Err("For every_hours, give hours from 1 to 168.".into());
            }
            Schedule::Every { hours: hours as u32 }
        }
        other => return Err(format!("\"{other}\" isn't a choice for when: use once, daily, weekdays, days or every_hours.")),
    };
    let notify = if text("notify") == Some("worth") { Notify::Worth } else { Notify::Always };
    let mut summary = format!("\"{name}\", {}", routines::describe(&schedule, &tz));
    // Soon: say how soon too, so a wrong time is easy to spot.
    if let Schedule::Once { at } = schedule {
        if at - now < 48 * 3600 {
            summary.push_str(&format!(" ({})", from_now(at - now)));
        }
    }
    Ok(Wanted {
        summary,
        input: RoutineInput {
            name: Some(name.chars().take(80).collect()),
            persona_id,
            instructions: Some(instructions.into()),
            schedule: Some(schedule),
            notify: Some(notify),
            time_zone,
            utc_offset_minutes,
            ..Default::default()
        },
    })
}

/// Make the routine (once the person said yes). The text for the model.
pub async fn make(ctx: &Context, w: Wanted) -> Result<String, String> {
    let r = routines::create(&ctx.state.pool, &ctx.account_id, w.input).await.map_err(|e| e.to_string())?;
    let next = r
        .next_at
        .and_then(|t| jiff::Timestamp::from_second(t).ok())
        .map(|t| format!(" It runs first {}.", t.to_zoned(zone(ctx)).strftime("%a %-d %b at %H:%M")))
        .unwrap_or_default();
    Ok(format!(
        "Made the routine {}.{next} The user is notified with its result. They can change, pause or delete it in Routines.",
        w.summary
    ))
}

/// The person's routines, for the model.
pub async fn list(ctx: &Context) -> Result<String, String> {
    let all = routines::list(&ctx.state.pool, &ctx.account_id).await.map_err(|e| e.to_string())?;
    if all.is_empty() {
        return Ok("The user has no routines.".into());
    }
    let tz = zone(ctx);
    let lines: Vec<String> = all
        .iter()
        .map(|r| {
            let state = if !r.enabled {
                "paused".to_string()
            } else {
                match r.next_at.and_then(|t| jiff::Timestamp::from_second(t).ok()) {
                    Some(t) => format!("next {}", t.to_zoned(tz.clone()).strftime("%a %-d %b %H:%M")),
                    None => "nothing scheduled".into(),
                }
            };
            let last = match (&r.last_status, &r.last_note) {
                (Some(s), Some(n)) => format!("; last run {s}: {}", n.chars().take(120).collect::<String>()),
                (Some(s), None) => format!("; last run {s}"),
                _ => String::new(),
            };
            format!(
                "- {} ({}; {state}{last}): {}",
                r.name,
                routines::describe(&r.schedule, &r.zone()),
                r.instructions.chars().take(160).collect::<String>()
            )
        })
        .collect();
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sydney(args: Value) -> Result<Wanted, String> {
        // Fri 6 Nov 2026, 20:00 in Sydney.
        parse(Some("Australia/Sydney".into()), Some(660), None, &args, "", 1_793_955_600)
    }

    #[test]
    fn in_so_many_minutes_is_read_from_what_the_person_said() {
        assert_eq!(said_in_minutes("Remind me in 1 minute to stretch"), Some(1));
        assert_eq!(said_in_minutes("remind me in an hour"), Some(60));
        assert_eq!(said_in_minutes("in half an hour, tell me to check the oven"), Some(30));
        assert_eq!(said_in_minutes("10 mins from now remind me"), Some(10));
        assert_eq!(said_in_minutes("ping me in 5min"), Some(5));
        assert_eq!(said_in_minutes("in 2 hours"), Some(120));
        assert_eq!(said_in_minutes("remind me at 5 to call Mum"), None);
        assert_eq!(said_in_minutes("every 6 hours check the page"), None);
        assert_eq!(said_in_minutes("in 5 minutes, and again in 10 minutes"), None, "two times: the model decides");
        // The model's sums are overruled.
        let args = json!({ "name": "Stretch", "instructions": "Remind the user to stretch.", "when": "once", "at": "2026-11-07 03:00" });
        let w = parse(Some("Australia/Sydney".into()), Some(660), None, &args, "remind me in 1 minute to stretch", 1_793_955_600).unwrap();
        assert_eq!(w.input.schedule, Some(Schedule::Once { at: 1_793_955_600 + 60 }));
        assert_eq!(w.summary, "\"Stretch\", once, Fri 6 Nov 20:01 (in 1 minute)");
    }

    #[test]
    fn routines_asked_for_in_chat_are_read_in_the_persons_time() {
        let w = sydney(json!({ "name": "Call Mum", "instructions": "Remind the user to call Mum.", "when": "once", "in_minutes": 20 })).unwrap();
        assert_eq!(w.input.schedule, Some(Schedule::Once { at: 1_793_955_600 + 1200 }));
        assert_eq!(w.summary, "\"Call Mum\", once, Fri 6 Nov 20:20 (in 20 minutes)");
        let w = sydney(json!({ "name": "Bins", "instructions": "Remind me.", "when": "once", "at": "2026-11-07 17:00" })).unwrap();
        assert_eq!(w.summary, "\"Bins\", once, Sat 7 Nov 17:00 (in 21 hours)");
        let w = sydney(json!({ "name": "Brief", "instructions": "Weather.", "when": "weekdays", "time": "7:30" })).unwrap();
        assert_eq!(w.input.schedule, Some(Schedule::Days { time: "07:30".into(), days: vec![1, 2, 3, 4, 5] }));
        let w = sydney(json!({ "name": "Gym", "instructions": "Go.", "when": "days", "time": "18:00", "days": ["Mon", "thursday"] })).unwrap();
        assert_eq!(w.summary, "\"Gym\", Mon, Thu at 18:00");
        let w = sydney(json!({ "name": "Paper", "instructions": "Check.", "when": "every_hours", "hours": 6, "notify": "worth" })).unwrap();
        assert_eq!(w.input.notify, Some(Notify::Worth));
        // What's wrong is said, so the model can fix it.
        assert!(sydney(json!({ "name": "Late", "instructions": "x", "when": "once", "at": "2026-11-06 19:00" })).unwrap_err().contains("passed"));
        assert!(sydney(json!({ "name": "x", "instructions": "x", "when": "daily" })).is_err());
        assert!(sydney(json!({ "name": "x", "instructions": "x", "when": "every_hours", "hours": 0 })).is_err());
        assert!(sydney(json!({ "instructions": "x", "when": "daily", "time": "07:00" })).is_err());
    }
}
