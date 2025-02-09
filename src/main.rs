/*
 * Copyright (c) 2024 Gates Solutions LLC.
 *
 *      This file is part of podpingd.
 *
 *     podpingd is free software: you can redistribute it and/or modify it under the terms of the GNU Lesser General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
 *
 *     podpingd is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more details.
 *
 *     You should have received a copy of the GNU Lesser General Public License along with podpingd. If not, see <https://www.gnu.org/licenses/>.
 */

mod config;
mod hive;
mod syncer;
mod writer;

use crate::config::{WriterType, CARGO_PKG_VERSION};
use crate::hive::jsonrpc::client::JsonRpcClientImpl;
use crate::syncer::Syncer;
use crate::writer::console_writer::ConsoleWriter;
use crate::writer::disk_writer::DiskWriter;
use crate::writer::object_storage_writer::ObjectStorageWriter;
use color_eyre::eyre::Result;
use tracing::{info, warn, Level};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use tokio::time::{sleep, Duration};
// for historical purposes
//const FIRST_PODPING_BLOCK: u64 = 53_691_004;

// Define a struct that represents a blockchain event
#[derive(Serialize)]
struct HiveEvent {
    action: String,
    data: String,
}

// Dummy async function simulating event retrieval from the Hive blockchain
async fn listen_to_event() -> HiveEvent {
    // Replace with your actual logic to fetch and process events from the Hive blockchain
    // For demonstration, we simulate a delay and then return a dummy event.
    sleep(Duration::from_secs(5)).await;
    HiveEvent {
        action: "new_post".to_string(),
        data: "This is simulated event data.".to_string(),
    }
}

#[tokio::main]
async fn main() -> Result<(), reqwest::Error> {
    color_eyre::install()?;

    let settings = config::load_config();

    let log_level = match settings.debug {
        false => Level::INFO,
        true => Level::DEBUG,
    };

    //let log_level = Level::ERROR;

    tracing_subscriber::fmt()
        .event_format(tracing_subscriber::fmt::format())
        .with_max_level(log_level)
        .with_target(false)
        .init();

    // JSON formatting throwing an error with fields from external libraries
    /*tracing_subscriber::fmt()
    .event_format(tracing_subscriber::fmt::format::json().flatten_event(true))
    .with_max_level(log_level)
    .with_target(false)
    .init();*/

    //let span = span!(Level::INFO, "main").entered();

    let version = CARGO_PKG_VERSION.unwrap_or("VERSION_NOT_FOUND");
    info!("{}", format!("Starting podpingd version {}", version));

    match settings.writer.enabled {
        true => {
            match settings.writer.type_ {
                Some(WriterType::Disk) => {
                    info!("Writing podpings to the local disk.");
                    let syncer = Syncer::<JsonRpcClientImpl, DiskWriter>::new(&settings).await?;

                    syncer.start().await?;
                }
                Some(WriterType::ObjectStorage) => {
                    info!("Writing podpings to object storage.");
                    let syncer =
                        Syncer::<JsonRpcClientImpl, ObjectStorageWriter>::new(&settings).await?;

                    syncer.start().await?;
                }
                None => {
                    panic!("Writer Type not set correctly!")
                }
            };
        }
        false => {
            if !settings.writer.disable_persistence_warnings {
                warn!("The persistent writer is disabled in settings!");

                if settings.scanner.start_block.is_some()
                    || settings.scanner.start_datetime.is_some()
                {
                    warn!("A start block/date is set.  Without persistence, the scan will start at the values *every time*.")
                }
            }

            info!("Writing podpings to the console.");

            let syncer = Syncer::<JsonRpcClientImpl, ConsoleWriter>::new(&settings).await?;

            syncer.start().await?;
        }
    }

    let client = Client::new();
    let target_endpoint = "http://example.com/api/podping";

    loop {
        // Listen for a new Hive blockchain event
        let event = listen_to_event().await;

        // Create the JSON payload, here using serde_json::json macro. You can also serialize using event directly.
        let payload = json!({
            "action": event.action,
            "data": event.data,
        });

        // Send a POST request with the event as JSON payload
        match client.post(target_endpoint)
            .json(&payload)
            .send()
            .await {
            Ok(response) => {
                println!("HTTP status: {}", response.status());
                // Additional error handling based on status code can be done here.
            }
            Err(error) => {
                eprintln!("HTTP request failed: {}", error);
                // Optionally retry or handle error accordingly.
            }
        }
    }

    //span.exit();

    Ok(())
}
