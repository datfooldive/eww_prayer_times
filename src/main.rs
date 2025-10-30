use chrono::{DateTime, Datelike, Local, NaiveTime, TimeZone};
use clap::Parser;
use notify_rust::Notification;
use rust_embed::RustEmbed;
use salah::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

#[derive(Serialize)]
struct PrayerOutput {
    #[serde(rename = "Fajr")]
    fajr: String,
    #[serde(rename = "Dhuhr")]
    dhuhr: String,
    #[serde(rename = "Asr")]
    asr: String,
    #[serde(rename = "Maghrib")]
    maghrib: String,
    #[serde(rename = "Isha")]
    isha: String,
    next: String,
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(long)]
    city: Option<String>,

    #[clap(long)]
    coordinate: Option<String>,

    #[clap(long, hide = true)]
    test_at: Option<String>,
}

#[derive(RustEmbed)]
#[folder = "src/"]
#[include = "cities.json"]
struct Asset;

#[derive(Deserialize, Debug)]
struct City {
    name: String,
    #[serde(deserialize_with = "deserialize_f64")]
    lat: f64,
    #[serde(deserialize_with = "deserialize_f64")]
    lon: f64,
}

fn deserialize_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Deserialize, Error};
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => s.parse().map_err(D::Error::custom),
        serde_json::Value::Number(n) => {
            n.as_f64().ok_or_else(|| D::Error::custom("Invalid number"))
        }
        _ => Err(D::Error::custom("Expected string or number")),
    }
}

static CITIES: OnceLock<Vec<City>> = OnceLock::new();

fn get_cities() -> Result<&'static Vec<City>, Box<dyn std::error::Error>> {
    if let Some(cities) = CITIES.get() {
        return Ok(cities);
    }

    let cities_file = Asset::get("cities.json").ok_or("cities.json must exist")?;
    let cities_json = std::str::from_utf8(cities_file.data.as_ref())?;
    let cities: Vec<City> = serde_json::from_str(cities_json)?;
    let _ = CITIES.set(cities);

    Ok(CITIES.get().expect("CITIES should be initialized"))
}

fn get_coordinates_from_city(city_name: &str) -> Result<Coordinates, Box<dyn std::error::Error>> {
    let cities = get_cities()?;
    let city = cities
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(city_name))
        .ok_or_else(|| format!("City '{}' not found in the local database.", city_name))?;
    Ok(Coordinates::new(city.lat, city.lon))
}

fn parse_coordinates(coordinate_str: &str) -> Result<Coordinates, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = coordinate_str.split(',').collect();
    if parts.len() != 2 {
        return Err("Invalid coordinate format. Use `lat,lon`".into());
    }
    let lat = parts[0].trim().parse::<f64>()?;
    let lon = parts[1].trim().parse::<f64>()?;
    Ok(Coordinates::new(lat, lon))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments to get city or coordinates.
    let cli = Cli::parse();

    let coords = if let Some(city) = cli.city {
        get_coordinates_from_city(&city)?
    } else if let Some(coordinate_str) = cli.coordinate {
        parse_coordinates(&coordinate_str)?
    } else {
        return Err("Please provide either --city or --coordinate".into());
    };

    // Determine if we are in test mode and get the fake "now".
    let test_now: Option<DateTime<Local>> = if let Some(test_at_str) = &cli.test_at {
        // The format is now just "HH:MM"
        let time = NaiveTime::parse_from_str(test_at_str, "%H:%M")?;
        let today = Local::now().date_naive();
        let naive_dt = today.and_time(time);
        Some(
            Local
                .from_local_datetime(&naive_dt)
                .single()
                .ok_or("Ambiguous or invalid time provided for --test-at")?,
        )
    } else {
        None
    };

    // Main loop to run continuously as a daemon, or once if in test mode.
    loop {
        // --- Calculate Prayer Times ---
        // Use the fake time if in test mode, otherwise use the real current time.
        let now = test_now.unwrap_or_else(Local::now);
        let local_date = now.date_naive();
        let configuration = Configuration::with(Method::Singapore, Madhab::Shafi);
        let prayers = PrayerSchedule::new()
            .on(local_date)
            .for_location(coords)
            .with_configuration(configuration)
            .calculate()?;

        let fajr_time = prayers.time(Prayer::Fajr).with_timezone(&Local);
        let dhuhr_time = prayers.time(Prayer::Dhuhr).with_timezone(&Local);
        let asr_time = prayers.time(Prayer::Asr).with_timezone(&Local);
        let maghrib_time = prayers.time(Prayer::Maghrib).with_timezone(&Local);
        let isha_time = prayers.time(Prayer::Isha).with_timezone(&Local);

        // --- Determine Next Prayer ---
        let prayer_times = [
            (Prayer::Fajr, fajr_time),
            (Prayer::Dhuhr, dhuhr_time),
            (Prayer::Asr, asr_time),
            (Prayer::Maghrib, maghrib_time),
            (Prayer::Isha, isha_time),
        ];

        let next_prayer_info = prayer_times.iter().find(|(_, time)| *time > now);

        // --- Output JSON to Stdout for eww ---
        let next_prayer_name_for_json = next_prayer_info
            .map(|(prayer, _)| format!("{:?}", prayer))
            .unwrap_or_else(|| "Fajr".to_string());

        let output_struct = PrayerOutput {
            fajr: fajr_time.format("%H:%M").to_string(),
            dhuhr: dhuhr_time.format("%H:%M").to_string(),
            asr: asr_time.format("%H:%M").to_string(),
            maghrib: maghrib_time.format("%H:%M").to_string(),
            isha: isha_time.format("%H:%M").to_string(),
            next: next_prayer_name_for_json,
        };

        {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            serde_json::to_writer(&mut handle, &output_struct)?;
            writeln!(&mut handle)?;
        }

        // --- Sleep Until Next Prayer and Notify ---
        if let Some((prayer, time)) = next_prayer_info {
            let sleep_duration = (*time - now).to_std().unwrap_or(Duration::from_secs(0));

            if test_now.is_some() {
                eprintln!(
                    "[TEST MODE] Sleeping for {:?} until {:?}.",
                    sleep_duration, prayer
                );
            }

            thread::sleep(sleep_duration);

            let prayer_name_str = format!("{:?}", prayer);
            let summary = format!("Waktu Sholat {}", prayer_name_str);
            let body = format!("Saatnya menunaikan sholat {}", prayer_name_str);
            Notification::new().summary(&summary).body(&body).show()?;

            thread::sleep(Duration::from_secs(1));
        } else {
            let tomorrow = local_date.succ_opt().unwrap();
            let midnight_local = Local
                .with_ymd_and_hms(tomorrow.year(), tomorrow.month(), tomorrow.day(), 0, 0, 1)
                .unwrap();
            let sleep_duration = (midnight_local - now).to_std()?;

            if test_now.is_some() {
                eprintln!(
                    "[TEST MODE] No more prayers today. Sleeping for {:?}.",
                    sleep_duration
                );
            }

            thread::sleep(sleep_duration);
        }

        // If we were in test mode, exit the loop after one run.
        if test_now.is_some() {
            break;
        }
    }

    Ok(())
}
