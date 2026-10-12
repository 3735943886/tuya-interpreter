//! Interactive switch: rustuya handles TCP; tuya-interpreter handles device semantics.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use rustuya::tokio::{Device, Event, Listener, Version};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tuya_interpreter::{Channel, Driver, DriverOptions, Input, Output, RulePack, merge_rules};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(about = "Interactive Tuya switch using rustuya and tuya-interpreter")]
struct Args {
    /// Private device JSON/list; omitted: included fictional switch for tuyamock.
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    device_id: Option<String>,
    #[arg(long)]
    ip: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    version: Option<String>,
    #[arg(long, default_value = "switch_1")]
    property: String,
    /// Official rules directory; defaults to this checkout's rules/builtin.
    #[arg(long)]
    rules_dir: Option<PathBuf>,
    /// Additional rule layers applied after the official pack.
    #[arg(long)]
    rules_layer: Vec<PathBuf>,
}

fn load_device(path: &Path, id: Option<&str>) -> Result<Value> {
    let payload: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let devices = match &payload {
        Value::Array(devices) => devices.clone(),
        Value::Object(_) if payload.get("id").is_some() => vec![payload],
        Value::Object(_) => payload["devices"]
            .as_array()
            .ok_or("Expected a device list")?
            .clone(),
        _ => return Err("Expected a device object or list".into()),
    };
    let mut selected: Vec<_> = devices
        .into_iter()
        .filter(|d| id.is_none_or(|id| d["id"] == id))
        .collect();
    if selected.len() != 1 {
        return Err("Select exactly one device with --device-id".into());
    }
    Ok(selected.remove(0))
}

fn load_rules(dir: &Path) -> Result<Arc<RulePack>> {
    let mut paths = std::fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    let layers = paths
        .into_iter()
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| e == "json")
                && p.file_name().is_some_and(|n| n != "manifest.json")
        })
        .map(|p| Ok(serde_json::from_slice(&std::fs::read(p)?)?))
        .collect::<Result<Vec<Value>>>()?;
    Ok(Arc::new(merge_rules(None, layers, false)?.pack))
}

fn version(value: &str) -> Result<Version> {
    Ok(match value {
        "3.1" => Version::V3_1,
        "3.2" => Version::V3_2,
        "3.3" => Version::V3_3,
        "3.4" => Version::V3_4,
        "3.5" => Version::V3_5,
        _ => return Err("Protocol version must be 3.1 through 3.5".into()),
    })
}

/// Commands are fire-and-forget in rustuya; their replies arrive on a listener.
async fn receive_dps(events: &mut Listener, expected: Option<&Value>) -> Result<Value> {
    let receive = async {
        loop {
            match events.recv().await {
                Some(Event::Frame(frame)) => {
                    if frame.payload.is_empty() {
                        continue;
                    }
                    let payload: Value = serde_json::from_slice(&frame.payload)?;
                    if payload.get("Error").is_some() || payload.get("Err").is_some() {
                        return Err("Device returned a protocol error".into());
                    }
                    if let Some(dps) = payload
                        .get("dps")
                        .or_else(|| payload.get("data").and_then(|d| d.get("dps")))
                    {
                        if !dps.is_object() {
                            return Err("Invalid DPS map".into());
                        }
                        if expected.is_some_and(|expected| {
                            expected
                                .as_object()
                                .unwrap()
                                .iter()
                                .any(|(dp, value)| dps.get(dp) != Some(value))
                        }) {
                            continue;
                        }
                        return Ok(dps.clone());
                    }
                }
                Some(Event::Lagged(_)) => {
                    return Err("Transport listener lost frames; retry read".into());
                }
                None => return Err("Device connection closed".into()),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), receive).await?
}

struct Session {
    driver: Driver,
    device: Device,
    started: Instant,
    property: String,
}

impl Session {
    async fn handle(&mut self, input: Input) -> Result<()> {
        for effect in self
            .driver
            .handle(self.started.elapsed().as_secs_f64(), input)?
        {
            match effect {
                Output::Descriptor { desc } => {
                    println!("Device: {}", desc["label"].as_str().unwrap_or("Switch"))
                }
                Output::Value { prop, value } => println!("{prop} = {value}"),
                Output::Reject { prop, code, reason } => {
                    println!("Rejected {prop}: {code:?} {reason}")
                }
                Output::SendMessage { json, .. } => {
                    println!("Sending DPS: {}", json["dps"]);
                    // Subscribe before sending; ignore older reports until the write is reflected.
                    let mut events = self.device.listener();
                    self.device.set_dps(json["dps"].clone()).await?;
                    let response = match receive_dps(&mut events, Some(&json["dps"])).await {
                        Ok(response) => response,
                        Err(error) if error.is::<tokio::time::error::Elapsed>() => {
                            // Firmware may acknowledge without reporting state.
                            self.device.query().await?;
                            receive_dps(&mut events, Some(&json["dps"])).await?
                        }
                        Err(error) => return Err(error),
                    };
                    for effect in self.driver.handle(
                        self.started.elapsed().as_secs_f64(),
                        Input::Message {
                            channel: Channel::Passive,
                            json: response,
                        },
                    )? {
                        if let Output::Value { prop, value } = effect {
                            println!("{prop} = {value}");
                        }
                    }
                }
                // This focused Boolean-switch example has no timer/settings programs.
                Output::SetTimer { .. } | Output::SettingsChanged { .. } => {
                    return Err(
                        "This switch example does not execute timers or persist settings".into(),
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn read(&mut self) -> Result<()> {
        // Subscribe before query so a fast reply cannot race the subscription.
        let mut events = self.device.listener();
        self.device.query().await?;
        let dps = receive_dps(&mut events, None).await?;
        if dps.as_object().is_none_or(|d| d.is_empty()) {
            return Err("Status response contains no DPS".into());
        }
        self.handle(Input::Message {
            channel: Channel::State,
            json: dps,
        })
        .await?;
        println!(
            "{}: {}",
            self.property,
            self.driver.values()[&self.property]
        );
        Ok(())
    }

    fn show(&self) {
        println!("Property / type / access / value / unit");
        for (prop, spec) in self.driver.descriptor()["props"].as_object().unwrap() {
            let access = if spec["rw"] == true {
                "read/write"
            } else {
                "read only"
            };
            println!(
                "{prop}: {} / {access} / {} {}",
                spec["type"].as_str().unwrap_or("unknown"),
                self.driver.values()[prop],
                spec["unit"].as_str().unwrap_or("")
            );
            if spec["rw"] == true {
                println!("  set {prop} <JSON value>");
                if spec["type"] == "binary" {
                    println!("  toggle {prop}");
                }
            }
        }
    }

    async fn run(&mut self) -> Result<()> {
        self.device.wait_connected(Duration::from_secs(10)).await?;
        self.handle(Input::Connected).await?;
        self.read().await?;
        self.show();
        println!(
            "Commands: props, set <property> <JSON>, toggle [property], on, off, read, describe, quit"
        );
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        loop {
            print!("tuya> ");
            io::stdout().flush()?;
            let line = tokio::select! {
                line = lines.next_line() => line?,
                _ = tokio::signal::ctrl_c() => { println!(); break; }
            };
            let Some(line) = line else { break };
            let mut parts = line.trim().splitn(3, char::is_whitespace);
            let command = parts.next().unwrap_or("");
            match command {
                "quit" | "exit" | "q" => break,
                "read" => {
                    self.read().await?;
                    self.show();
                }
                "props" | "help" => self.show(),
                "set" => {
                    if let (Some(prop), Some(raw)) = (parts.next(), parts.next()) {
                        match serde_json::from_str(raw.trim()) {
                            Ok(value) => {
                                self.handle(Input::Command {
                                    prop: prop.into(),
                                    value,
                                })
                                .await?;
                                self.read().await?;
                                self.show();
                            }
                            Err(_) => println!("Value must be JSON, e.g. true or 23.5"),
                        }
                    } else {
                        println!("Usage: set <property> <JSON value>");
                    }
                }
                "describe" => println!(
                    "{}",
                    serde_json::to_string_pretty(self.driver.descriptor())?
                ),
                "on" | "off" | "toggle" => {
                    let prop = if command == "toggle" {
                        parts.next().unwrap_or(&self.property).to_owned()
                    } else {
                        self.property.clone()
                    };
                    let spec = &self.driver.descriptor()["props"][&prop];
                    if spec["type"] != "binary" || spec["rw"] != true {
                        println!("Select a writable binary property shown by props");
                        continue;
                    }
                    let value = if command == "toggle" {
                        self.read().await?;
                        !self.driver.values()[&prop]
                            .as_bool()
                            .ok_or("Switch state is unknown")?
                    } else {
                        command == "on"
                    };
                    self.handle(Input::Command {
                        prop,
                        value: json!(value),
                    })
                    .await?;
                    self.read().await?;
                }
                "" => {}
                _ => println!(
                    "Commands: props, set <property> <JSON>, toggle [property], on, off, read, describe, quit"
                ),
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let sample = args.file.is_none();
    let metadata = load_device(
        &args
            .file
            .unwrap_or_else(|| examples.join("devices/toggle-switch.json")),
        args.device_id.as_deref(),
    )?;
    if metadata["sub"] == true {
        return Err("Gateway subdevices are outside this example's scope".into());
    }
    let ip = args
        .ip
        .or_else(|| metadata["ip"].as_str().map(str::to_owned))
        .or_else(|| sample.then(|| "127.0.0.1".into()))
        .ok_or("Provide --ip or ip in the device JSON")?;
    let wire_version = args
        .version
        .or_else(|| metadata["version"].as_str().map(str::to_owned))
        .or_else(|| metadata["version"].as_f64().map(|v| v.to_string()))
        .or_else(|| sample.then(|| "3.5".into()))
        .ok_or("Provide --version or version in the device JSON")?;
    let key = std::env::var("TUYA_LOCAL_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .or_else(|| {
            metadata["local_key"]
                .as_str()
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| sample.then(|| "0123456789abcdef".into()))
        .ok_or("Provide TUYA_LOCAL_KEY or local_key in private JSON")?;
    let rules_dir = args
        .rules_dir
        .unwrap_or_else(|| examples.join("../rules/builtin"));
    let base = load_rules(&rules_dir)?;
    let layers = args
        .rules_layer
        .iter()
        .map(|path| Ok(serde_json::from_slice(&std::fs::read(path)?)?))
        .collect::<Result<Vec<Value>>>()?;
    let rules = Arc::new(merge_rules(Some(&base), layers, false)?.pack);
    let driver = Driver::new(metadata.clone(), rules, DriverOptions::default())?;
    let property = &driver.descriptor()["props"][&args.property];
    if property["type"] != "binary" || property["rw"] != true {
        return Err("--property must select a writable binary IL property".into());
    }
    let device = Device::builder(
        metadata["id"].as_str().ok_or("Device needs id")?,
        key.into_bytes(),
    )
    .address(ip)
    .port(args.port.unwrap_or(if sample { 16668 } else { 6668 }))
    .version(version(&wire_version)?)
    .auto_reconnect(false)
    .connect()?;
    let mut session = Session {
        driver,
        device,
        started: Instant::now(),
        property: args.property,
    };
    let result = session.run().await;
    // Close transport on EOF, quit, Ctrl-C and failures.
    let _ = session.handle(Input::Disconnected).await;
    session.device.close().await;
    result
}
