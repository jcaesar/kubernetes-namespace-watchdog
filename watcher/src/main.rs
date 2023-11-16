use futures::future::Either;
use futures::stream;
use futures::stream::StreamExt;
use futures::Stream;
use k8s_openapi::api::core::v1::Namespace;
use k8s_openapi::chrono::Local;
use kube::api::DeleteParams;
use kube::Api;
use kube::Client;
use kube::Config;
use kubernetes_namespace_watchdog_lib::PostTimeout;
use kubernetes_namespace_watchdog_lib::WatchArgs;
use std::cmp::max;
use std::cmp::min;
use std::convert::Infallible;
use std::time::Duration;
use std::time::Instant;
use tokio::select;
use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;
use tokio::sync::mpsc;
use tokio::time::sleep_until;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::wrappers::SignalStream;
use warp::Filter;

/// Watchdog: Remove a namespace if no treat is fed to us for a while
/// Not meant to be run manually
#[derive(clap::Parser)]
#[command(author, version, about)]
struct Main {
    #[command(flatten)]
    command: WatchArgs,
}

const FMT: &str = "%Y-%m-%d %H:%M:%S";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let command: Main = clap::Parser::parse();
    let WatchArgs {
        max_timeout,
        initial_timeout,
        sigusr1_timeout,
        listen,
    } = command.command;
    let client_config = &Config::infer()
        .await
        .expect("Failed to configure kube client");
    let client = &Client::try_from(client_config.clone()).expect("Failed to create kube client");
    match client_config.auth_info.token_file.as_deref() {
        Some("/var/run/secrets/kubernetes.io/serviceaccount/token") => (),
        _ => eprintln!(concat!(
            "WARNING: It seems we're not running in a K8s pod with a service account.",
            " If you're running this manually, hit Ctrl-C now!",
        )),
    }
    let max_timeout = max_timeout.unwrap_or(initial_timeout);
    let sigusr1_timeout = sigusr1_timeout.map(|sigusr1_timeout| {
        SignalStream::new(signal(SignalKind::user_defined1()).expect("Failed to listen for signal"))
            .map(move |()| {
                heartbeat("USR1", sigusr1_timeout);
                Instant::now() + sigusr1_timeout
            })
    });
    let listen_timeout = listen.map(|listen| {
        let (ps, pr) = mpsc::channel(10);
        let route = warp::any()
            .and(warp::filters::method::post())
            .and(warp::query::<PostTimeout>())
            .and(warp::any().map(move || ps.clone()))
            .and_then(
                move |PostTimeout { timeout }: PostTimeout, ps: mpsc::Sender<Instant>| async move {
                    let adv = min(max_timeout, timeout.unwrap_or(initial_timeout));
                    let timeout = Instant::now() + adv;
                    heartbeat("HTTP", adv);
                    ps.send(timeout).await.expect("Channel died");
                    Ok::<_, Infallible>("bumped")
                },
            );
        tokio::spawn(warp::serve(route).run(listen));
        ReceiverStream::new(pr)
    });
    let mut timeouts = tokio_stream::StreamExt::merge(
        none_to_pending(sigusr1_timeout),
        none_to_pending(listen_timeout),
    );
    let mut deadline = Instant::now() + initial_timeout;
    loop {
        select! {
            timeout = timeouts.next() => {
                deadline = max(deadline, timeout.expect("Endless stream ended"));
            }
            () = sleep_until(deadline.into()) => {
                Api::<Namespace>::all(client.clone())
                    .delete(&client_config.default_namespace, &DeleteParams::default())
                    .await.expect("Failed to delete namespace");
                eprintln!("We should have just deleted ourselves. Yet we're still here. Time will heal that.");
                break;
            }
        }
    }
}

fn heartbeat(bumper: &str, adv: Duration) {
    let now = Local::now();
    println!(
        "[{}] {bumper} bumped to {}",
        now.format(FMT),
        (now + adv).format(FMT)
    );
}

fn none_to_pending<T, S: Stream<Item = T>>(ose: Option<S>) -> impl Stream<Item = T> {
    match ose {
        Some(ose) => Either::Right(ose),
        None => Either::Left(stream::pending()),
    }
}
