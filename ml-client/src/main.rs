use reqwest::{
    header::{HeaderMap, HeaderValue},
    Url,
};
use serde::Deserialize;
use std::{
    env::args,
    fs,
    io::stdin,
    process::{Command, Stdio},
};

#[derive(Deserialize)]
struct Config {
    apikey: String,
    auth_address: String,
    mc_address: String,
    python: String,
    work_dir: String,
    main_dir: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = args().skip(1);
    let path = args.next().unwrap_or("client-config.ron".to_string());
    let config: Config = ron::de::from_str(&fs::read_to_string(path)?)?;

    let stdin = stdin();

    println!("Enter name:");

    let mut name = String::new();
    stdin.read_line(&mut name)?;
    let name = name.trim();

    let mut headers = HeaderMap::new();
    headers.insert("APIKey", HeaderValue::from_str(&config.apikey)?);

    let client = reqwest::blocking::Client::builder()
        .default_headers(headers)
        .build()?;

    match client.get(Url::parse(&config.auth_address)?).send() {
        Ok(response) => {
            if !response.status().is_success() {
                return Err(format!(
                    "Server replied with a non-OK error code: {}",
                    response.status()
                )
                .into());
            }
        }
        Err(why) => return Err(format!("Error sending request: {}", why).into()),
    }

    let mut handle = Command::new(config.python)
        .arg("-m")
        .arg("portablemc")
        .arg("--main-dir")
        .arg(config.main_dir)
        .arg("--work-dir")
        .arg(config.work_dir)
        .arg("start")
        .arg("-u")
        .arg(name)
        .arg("-s")
        .arg(config.mc_address)
        .stdout(Stdio::inherit())
        .stdin(Stdio::inherit())
        .spawn()?;

    handle.wait().unwrap();

    Ok(())
}
