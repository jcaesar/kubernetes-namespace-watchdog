use humantime_serde::re::humantime;
use serde::{Deserialize, Serialize};
use serde_with::DisplayFromStr;
use std::{net::SocketAddr, time::Duration};

#[serde_with::serde_as]
#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
pub struct WatchArgs {
    /// Initial timeout
    #[cfg_attr(feature = "clap", arg(long))]
    #[cfg_attr(feature = "clap", clap(value_parser = humantime::parse_duration))]
    #[serde(with = "humantime_serde")]
    pub initial_timeout: Duration,

    /// Maximum timeout
    #[cfg_attr(feature = "clap", arg(long))]
    #[cfg_attr(feature = "clap", clap(value_parser = humantime::parse_duration))]
    #[serde(with = "humantime_serde")]
    pub max_timeout: Option<Duration>,

    /// Reset timeout to this on SIGUSR1
    #[cfg_attr(feature = "clap", arg(long))]
    #[cfg_attr(feature = "clap", clap(value_parser = humantime::parse_duration))]
    #[serde(with = "humantime_serde")]
    pub sigusr1_timeout: Option<Duration>,

    /// HTTP API to reset timeout
    #[cfg_attr(feature = "clap", arg(long))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub listen: Option<SocketAddr>,
}
