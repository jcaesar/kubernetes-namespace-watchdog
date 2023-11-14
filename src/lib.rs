use serde_with::DisplayFromStr;
use std::{net::SocketAddr, time::Duration};

use serde::{Deserialize, Serialize};

#[serde_with::serde_as]
#[serde_with::skip_serializing_none]
#[derive(clap::Parser, Deserialize, Serialize)]
pub struct WatchArgs {
    /// Maximum timeout
    #[arg(long)]
    #[clap(value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub max_timeout: Option<Duration>,

    /// Initial timeout
    #[arg(long)]
    #[clap(value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub initial_timeout: Duration,

    /// Reset timeout to this on SIGUSR1
    #[arg(long)]
    #[clap(value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub sigusr1_timeout: Option<Duration>,

    /// HTTP API to reset timeout
    #[arg(long)]
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub listen: Option<SocketAddr>,
}
