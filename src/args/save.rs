use super::{InputType, Quality};
use anyhow::{bail, Result};
use clap::Args;
use kdam::term::Colorizer;
use reqwest::blocking::Client;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Proxy, Url};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Download HLS and Dash playlists.
/// 
/// Playlists which have separate audio tracks or subtitles streams, expects
/// ffmpeg (https://www.ffmpeg.org/download.html) to be installed in system PATH.
#[derive(Debug, Clone, Args)]
pub struct Save {
    /// URL | .m3u8 | .m3u | .mpd | .xml
    #[clap(required = true, validator = input_validator)]
    pub input: String,

    /// Path of final downloaded video stream.
    /// For file extension any ffmpeg supported format could be provided.
    /// If playlist contains alternative streams vsd will try to transmux and trancode into single file using ffmpeg.
    #[clap(short, long)]
    pub output: Option<String>,

    /// Base url for all segments.
    /// Usually needed for local m3u8 file.
    #[clap(short, long)]
    pub baseurl: Option<String>,

    /// Automatic selection of some standard resolution streams with highest bandwidth stream variant from master playlist.
    /// possible values: [144p, 240p, 360p, 480p, 720p, hd, 1080p, fhd, 2k, 1440p, qhd, 4k, 8k, highest, max, select-later]
    #[clap(short, long, default_value = "select-later", value_name = "WIDTHxHEIGHT", validator = quality_validator)]
    pub quality: String,

    // /// Automatic selection of some standard resolution streams with highest bandwidth stream variant from master playlist.
    // #[clap(long, number_of_values = 2, value_names = &["width", "height"])]
    // pub resolution: Vec<u64>,
    /// Maximum number of threads for parllel downloading of segments.
    /// Number of threads should be in range 1-16 (inclusive).
    #[clap(short, long, default_value_t = 5, validator = threads_validator)]
    pub threads: u8,

    /// Maximum number of retries to download an individual segment.
    #[clap(long, default_value_t = 15)]
    pub retry_count: u8,

    /// Raw style input prompts for old and unsupported terminals.
    #[clap(long)]
    pub raw_prompts: bool,

    /// Resume a download session.
    /// Download session can only be resumed if download session json file is present.
    #[clap(short, long)]
    pub resume: bool,

    /// Download alternative streams such as audio and subtitles streams from master playlist instead of variant video streams.
    #[clap(short, long)]
    pub alternative: bool,

    /// Skip downloading and muxing alternative streams.
    #[clap(short, long)]
    pub skip: bool,

    /// TODO: Decryption keys.
    /// This option can be used multiple times.
    #[clap(short, long, multiple_occurrences = true, value_name = "<KID:KEY>|KEY")]
    pub key: Vec<String>,

    /// TODO: Record duration for live playlist in seconds.
    #[clap(long)]
    pub record_duration: Option<f32>,

    /// TODO: Directory path
    #[clap(long)]
    pub save_directory: Option<String>,

    /// Custom headers for requests.
    /// This option can be used multiple times.
    #[clap(long, multiple_occurrences = true, number_of_values = 2, value_names = &["KEY", "VALUE"], help_heading = "CLIENT OPTIONS")]
    pub header: Vec<String>, // Vec<Vec<String>> not supported

    /// Update and set custom user agent for requests.
    #[clap(
        long,
        default_value = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/101.0.4951.64 Safari/537.36",
        help_heading = "CLIENT OPTIONS"
    )]
    pub user_agent: String,

    /// Set http or https proxy address for requests.
    #[clap(long, validator = proxy_address_validator, help_heading = "CLIENT OPTIONS")]
    pub proxy_address: Option<String>,

    /// Enable cookie store which allows cookies to be stored.
    #[clap(long, help_heading = "CLIENT OPTIONS")]
    pub enable_cookies: bool,

    /// Enable cookie store and fill it with some existing cookies.
    /// Example `--cookies "foo=bar; Domain=yolo.local" https://yolo.local`.
    /// This option can be used multiple times.
    #[clap(long, multiple_occurrences = true, number_of_values = 2, value_names = &["COOKIES", "URL"], help_heading = "CLIENT OPTIONS")]
    pub cookies: Vec<String>, // Vec<Vec<String>> not supported
}

fn input_validator(s: &str) -> Result<(), String> {
    if s.starts_with("https://youtube.com")
        || s.starts_with("https://www.youtube.com")
        || s.starts_with("https://youtu.be")
    {
        Err("Youtube links aren't supported yet".to_owned())
    } else {
        Ok(())
    }
}

fn quality_validator(s: &str) -> Result<(), String> {
    let _ = s.parse::<Quality>()?;
    Ok(())
}

fn threads_validator(s: &str) -> Result<(), String> {
    let num_threads: usize = s.parse().map_err(|_| format!("`{}` isn't a number", s))?;
    if std::ops::RangeInclusive::new(1, 16).contains(&num_threads) {
        Ok(())
    } else {
        Err("number of threads should be in range `1-16`".to_string())
    }
}

fn proxy_address_validator(s: &str) -> Result<(), String> {
    if s.starts_with("http://") || s.starts_with("https://") {
        Ok(())
    } else {
        Err("Proxy address should start with `http://` or `https://` only".to_string())
    }
}

impl Save {
    pub fn client(&self) -> Result<Arc<Client>> {
        let mut client_builder = Client::builder().user_agent(&self.user_agent);

        if !self.header.is_empty() {
            let mut headers = HeaderMap::new();

            for i in (0..headers.len()).step_by(2) {
                headers.insert(
                    self.header[i].parse::<HeaderName>()?,
                    self.header[i + 1].parse::<HeaderValue>()?,
                );
            }

            client_builder = client_builder.default_headers(headers);
        }

        if let Some(proxy) = &self.proxy_address {
            if proxy.starts_with("https") {
                client_builder = client_builder.proxy(Proxy::https(proxy)?);
            } else if proxy.starts_with("http") {
                client_builder = client_builder.proxy(Proxy::http(proxy)?);
            }
        }

        if self.enable_cookies || !self.cookies.is_empty() {
            client_builder = client_builder.cookie_store(true);
        }

        if !self.cookies.is_empty() {
            let jar = Jar::default();

            for i in (0..self.cookies.len()).step_by(2) {
                jar.add_cookie_str(&self.cookies[i], &self.cookies[i + 1].parse::<Url>()?);
            }

            client_builder = client_builder.cookie_provider(Arc::new(jar));
        }

        Ok(Arc::new(client_builder.build()?))
    }

    pub fn get_url(&self, uri: &str) -> Result<String> {
        if uri.starts_with("http") {
            Ok(uri.to_owned())
        } else if let Some(baseurl) = &self.baseurl {
            Ok(Url::parse(baseurl)?.join(uri)?.to_string())
        } else {
            if !self.input.starts_with("http") {
                bail!(
                    "Non HTTP input should have {} set explicitly.",
                    "--baseurl".colorize("bold green")
                )
            }

            Ok(Url::parse(&self.input)?.join(uri)?.to_string())
        }
    }

    pub fn tempfile(&self) -> String {
        let output = self
            .input
            .split('?')
            .next()
            .unwrap()
            .split('/')
            .last()
            .unwrap();

        let output = if output.ends_with(".m3u") || output.ends_with(".m3u8") {
            if output.ends_with(".ts.m3u8") {
                output.trim_end_matches(".m3u8").to_owned()
            } else {
                let mut path = PathBuf::from(&output);
                path.set_extension("ts");
                path.to_str().unwrap().to_owned()
            }
        } else if output.ends_with(".mpd") || output.ends_with(".xml") {
            let mut path = PathBuf::from(&output);
            path.set_extension("m4s");
            path.to_str().unwrap().to_owned()
        } else {
            let mut path = PathBuf::from(
                output
                    .replace('<', "-")
                    .replace('>', "-")
                    .replace(':', "-")
                    .replace('\"', "-")
                    .replace('/', "-")
                    .replace('\\', "-")
                    .replace('|', "-")
                    .replace('?', "-"),
            );
            path.set_extension("mp4");
            path.to_str().unwrap().to_owned()
        };

        if Path::new(&output).exists() && !self.resume {
            let stemed_path = Path::new(&output).file_stem().unwrap().to_str().unwrap();
            let ext = Path::new(&output).extension().unwrap().to_str().unwrap();

            for i in 1.. {
                let core_file_copy = format!("{} ({}).{}", stemed_path, i, ext);

                if !Path::new(&core_file_copy).exists() {
                    return core_file_copy;
                }
            }
        }

        output
    }

    pub fn input_type(&self) -> InputType {
        let url = self.input.split('?').next().unwrap();

        if url.starts_with("http") {
            if url.ends_with(".m3u") || url.ends_with(".m3u8") || url.ends_with("ts.m3u8") {
                InputType::HlsUrl
            } else if url.ends_with(".mpd") || url.ends_with(".xml") {
                InputType::DashUrl
            } else {
                InputType::Website
            }
        } else {
            if url.ends_with(".m3u") || url.ends_with(".m3u8") || url.ends_with("ts.m3u8") {
                InputType::HlsLocalFile
            } else if url.ends_with(".mpd") || url.ends_with(".xml") {
                InputType::DashLocalFile
            } else {
                InputType::LocalFile
            }
        }
    }
}