use std::fmt;

use std::os::unix::io::FromRawFd;
use std::time::Duration;

use futures::StreamExt;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::BoxedStream;

use serde::{de, Deserialize, Deserializer};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    Forward, // On my mouse, these map to forward and back
    Back,
    Unknown,
}

fn deserialize_mousebutton<'de, D>(deserializer: D) -> Result<MouseButton, D::Error>
where
    D: Deserializer<'de>,
{
    struct MouseButtonVisitor;

    impl<'de> de::Visitor<'de> for MouseButtonVisitor {
        type Value = MouseButton;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("u64")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // TODO: put this behind `--debug` flag
            //eprintln!("{}", value);
            Ok(match value {
                1 => MouseButton::Left,
                2 => MouseButton::Middle,
                3 => MouseButton::Right,
                4 => MouseButton::WheelUp,
                5 => MouseButton::WheelDown,
                9 => MouseButton::Forward,
                8 => MouseButton::Back,
                _ => MouseButton::Unknown,
            })
        }
    }

    deserializer.deserialize_any(MouseButtonVisitor)
}

#[derive(Deserialize, Debug, Clone)]
struct I3BarEventRaw {
    pub name: Option<String>,
    pub instance: Option<String>,
    #[serde(deserialize_with = "deserialize_mousebutton")]
    pub button: MouseButton,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct I3BarEvent {
    pub id: usize,
    pub instance: Option<usize>,
    pub button: MouseButton,
}

fn unprocessed_events_stream(invert_scrolling: bool) -> BoxedStream<I3BarEvent> {
    // Avoid spawning a blocking therad (why doesn't tokio do this too?)
    // This should be safe given that this function is called only once
    let stdin = unsafe { File::from_raw_fd(0) };
    let lines = BufReader::new(stdin).lines();

    futures::stream::unfold(lines, move |mut lines| async move {
        loop {
            // Take only the valid JSON object betweem curly braces (cut off leading bracket, commas and whitespace)
            let line = lines.next_line().await.ok().flatten()?;
            let line = line.trim_start_matches(|c| c != '{');
            let line = line.trim_end_matches(|c| c != '}');

            if line.is_empty() {
                continue;
            }

            let event: I3BarEventRaw = serde_json::from_str(line).unwrap();
            let id = match event.name {
                Some(name) => name.parse().unwrap(),
                None => continue,
            };
            let instance = event.instance.map(|x| x.parse::<usize>().unwrap());

            use MouseButton::*;
            let button = match (event.button, invert_scrolling) {
                (WheelUp, false) | (WheelDown, true) => WheelUp,
                (WheelUp, true) | (WheelDown, false) => WheelDown,
                (other, _) => other,
            };

            let event = I3BarEvent {
                id,
                instance,
                button,
            };

            break Some((event, lines));
        }
    })
    .boxed()
}

pub fn events_stream(
    invert_scrolling: bool,
    double_click_delay: Duration,
) -> BoxedStream<I3BarEvent> {
    let events = unprocessed_events_stream(invert_scrolling);
    futures::stream::unfold((events, None), move |(mut events, pending)| async move {
        if let Some(pending) = pending {
            return Some((pending, (events, None)));
        }

        let mut event = events.next().await?;

        // Handle double clicks (for now only left)
        // if event.button == MouseButton::Left && !double_click_delay.is_zero() {
        //     if let Ok(new_event) = tokio::time::timeout(double_click_delay, events.next()).await {
        //         let new_event = new_event?;
        //         if event == new_event {
        //             event.button = MouseButton::DoubleLeft;
        //         } else {
        //             return Some((event, (events, Some(new_event))));
        //         }
        //     }
        // }

        Some((event, (events, None)))
    })
    .boxed_local()
}
