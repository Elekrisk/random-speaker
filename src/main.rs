#![feature(try_blocks)]

use std::{
    io::BufReader,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

use chrono::{
    Date, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike, Utc, Weekday,
};
use rand::{seq::SliceRandom, thread_rng, Rng};
use rodio::{Decoder, OutputStream, Sink};
use serde::Deserialize;
use tokio::{
    select,
    time::{Instant, Sleep},
};

#[tokio::main]
async fn main() {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

    let sink = Sink::try_new(&stream_handle).unwrap();

    let mut context = Context {
        sink,
        config: BaseConfig::default(),
        sleep: Box::pin(tokio::time::sleep(Duration::MAX)),
    };

    context.run().await;
}

#[derive(Debug, Deserialize, Default)]
struct BaseConfig {
    general: General,
    schedule: Schedule,
}

#[derive(Debug, Deserialize, Default)]
struct General {
    lower_bound: usize,
    upper_bound: usize,
}

#[derive(Debug, Deserialize, Default)]
struct Schedule {
    weekdays: Vec<Weekday>,
    start_time: NaiveTime,
    end_time: NaiveTime,
}

struct Context {
    sink: Sink,
    config: BaseConfig,
    sleep: Pin<Box<Sleep>>,
}

impl Context {
    async fn run(&mut self) {
        let (mut watcher, mut channel) =
            async_watcher::AsyncDebouncer::new_with_channel(Duration::from_secs(1), None)
                .await
                .unwrap();
        watcher
            .watcher()
            .watch(
                Path::new("config.toml"),
                async_watcher::notify::RecursiveMode::NonRecursive,
            )
            .unwrap();

        self.wake();

        let mut i = 1;

        loop {
            select! {
                Some(event) = channel.recv() => {
                    match event {
                        Ok(events) => {
                            for event in events {
                                println!("{i} -- {event:?}");
                                i += 1;
                            }

                            watcher.watcher().watch(Path::new("config.toml"), async_watcher::notify::RecursiveMode::NonRecursive).unwrap();
                        }
                        Err(errors) => {
                            for err in errors {
                                println!("{err:?}");
                            }
                        }
                    }
                    self.wake();
                }
                _ = &mut self.sleep => {
                    // We should have now waited until the next play time
                    self.wake();
                }
                else => break
            }
        }
    }

    fn wake(&mut self) {
        // Update config from file
        self.config = match toml::from_str(&std::fs::read_to_string("config.toml").unwrap()) {
            Ok(val) => val,
            Err(e) => {
                eprintln!("Error reading config: {e}");
                return;
            }
        };

        // println!("{config:#?}");
        // println!("{}", Local::now().date_naive().weekday());

        // Check if we are waiting for a play event
        let next_play: anyhow::Result<NaiveDateTime> = try {
            let contents = std::fs::read_to_string("next-play")?;
            contents.trim().parse()?
        };

        match next_play {
            Ok(next_play) => {
                let diff = next_play.signed_duration_since(Local::now().naive_local());
                // println!("diff: {diff}");
                if diff < TimeDelta::zero() {
                    println!("Next play time reached {:.2} seconds ago", diff.abs());
                    // We should play sound and then schedule a new next-play
                    // First, check that the current time is valid
                    if self.is_time_valid(Local::now().naive_local()) {
                        println!("Play sound and reschedule");
                        self.play_sound();
                        self.schedule_new_play();
                    } else {
                        println!("Current time invalid, reschedule");
                        // Current time is not valid
                        // Possible causes:
                        // 1. We waited too long, and we just barely entered invalid time
                        // 2. We have been turned off and just started, within invalid time
                        // The easiest solution is to simply reschedule, as these should be pretty unusual circumstances
                        self.schedule_new_play();
                    }
                } else {
                    println!(
                        "Next play time not reached, waiting additional {} seconds",
                        next_play
                            .signed_duration_since(Local::now().naive_local())
                            .num_seconds()
                    );
                    // We should simply wait
                    self.sleep.as_mut().reset(
                        Instant::now()
                            + next_play
                                .signed_duration_since(Local::now().naive_local())
                                .to_std()
                                .unwrap_or_default(),
                    );
                }
            }
            Err(e) => {
                // Something went wrong
                // We could either not read a next-play file, or it is invalid
                // We should schedule a new next-play
                println!("Could not find and/or read next-play file, reschedule");
                self.schedule_new_play();
            }
        }
    }

    fn collect_sounds(&self, path: impl AsRef<Path>) -> Vec<AudioFile> {
        let mut res = vec![];
        let mut count = 0;
        for file in std::fs::read_dir(path).unwrap() {
            let file = file.unwrap();

            let file_type = file.file_type().unwrap();
            if file_type.is_file() {
                if file.file_name() == "config.toml" {
                    continue;
                }

                res.push(AudioFile {
                    path: file.path(),
                    config: FileConfig { weight: 1.0 },
                })
            } else if file_type.is_dir() {
                let mut sounds = self.collect_sounds(file.path());
                res.append(&mut sounds);
            }

            count += 1;
        }

        for file in &mut res {
            file.config.weight /= count as f32;
        }

        res
    }

    fn play_sound(&self) {
        let sounds = self.collect_sounds("sounds");
        let Ok(sound) = sounds.choose_weighted(&mut thread_rng(), |file| file.config.weight) else {
            eprintln!("No sound to play");
            return;
        };

        let source =
            Decoder::new(BufReader::new(std::fs::File::open(&sound.path).unwrap())).unwrap();
        self.sink.append(source);
        eprintln!(
            "Playing {}",
            sound
                .path
                .file_name()
                .map(|s| s.to_string_lossy())
                .unwrap_or("-- CANNOT GET FILE NAME --".into())
        );
    }

    fn is_time_valid(&self, time: NaiveDateTime) -> bool {
        self.config.schedule.weekdays.contains(&time.weekday())
            && (self.config.schedule.start_time..=self.config.schedule.end_time)
                .contains(&time.time())
    }

    fn find_last_valid_time(&self, mut time: NaiveDateTime) -> NaiveDateTime {
        if self.is_time_valid(time) {
            time
        } else {
            // Find last previous valid time

            if time.time() < self.config.schedule.start_time {
                time -= chrono::Duration::days(1);
            }
            while !self.config.schedule.weekdays.contains(&time.weekday()) {
                time -= chrono::Duration::days(1);
            }
            NaiveDateTime::new(time.date(), self.config.schedule.end_time)
        }
    }

    fn find_next_valid_time(&self, mut time: NaiveDateTime) -> NaiveDateTime {
        if self.is_time_valid(time) {
            time
        } else {
            // Find last previous valid time

            if time.time() > self.config.schedule.end_time {
                time += chrono::Duration::days(1);
            }
            while !self.config.schedule.weekdays.contains(&time.weekday()) {
                time += chrono::Duration::days(1);
            }
            NaiveDateTime::new(time.date(), self.config.schedule.start_time)
        }
    }

    fn schedule_new_play(&mut self) {
        let mut current_time = Local::now().naive_local();

        // First, find out if the current time is a valid time.
        // If it isn't, we schedule our next play as if the last valid time is when the scheduling occured.
        // This allows the sound to be scheduled the same way no matter if we just started in the middle of the night
        // or we just played a sound, without sounds starting playing the instant we reach a valid time.
        current_time = self.find_last_valid_time(current_time);

        // Generate a new time for play
        let seconds_from_now = thread_rng().gen_range(
            self.config.general.lower_bound as f32..self.config.general.upper_bound as f32,
        );

        let mut then = current_time + Duration::from_secs_f32(seconds_from_now);

        // Check if the scheduled time is valid
        while !self.is_time_valid(then) {
            // The next scheduled time isn't valid, get how long after the last valid time it is scheduled
            let last_valid = self.find_last_valid_time(then);
            let diff = then.signed_duration_since(last_valid);
            // We reschedule the play, pretending that the invalid time period simply is cut out from reality
            then = self.find_next_valid_time(then) + diff;
        }

        println!("Next play @ {then}");

        // Write the next play to file, so that it survives speaker reboot
        std::fs::write("next-play", then.format("%Y-%m-%dT%H:%M:%S.%f\n").to_string()).unwrap();

        self.sleep_until(then);
    }

    fn sleep_until(&mut self, time: NaiveDateTime) {
        println!(
            "Sleeping until {}, which is {} seconds",
            time,
            time.signed_duration_since(Local::now().naive_local())
                .num_seconds()
        );
        // We should simply wait
        self.sleep.as_mut().reset(
            Instant::now()
                + time
                    .signed_duration_since(Local::now().naive_local())
                    .to_std()
                    .unwrap_or_default(),
        );
    }
}

enum DirectoryEntry {
    Directory(Directory),
    File(AudioFile),
}

struct Directory {
    path: PathBuf,
    config: DirectoryConfig,
}

struct DirectoryConfig {}

struct AudioFile {
    path: PathBuf,
    config: FileConfig,
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    #[serde(default = "default_weight")]
    weight: f32,
}

const fn default_weight() -> f32 {
    1.0
}
