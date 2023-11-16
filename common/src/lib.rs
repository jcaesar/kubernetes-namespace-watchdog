use anyhow::{ensure, Context};
use humantime_serde::re::humantime;
use hyper::Request;
use k8s_openapi::{
    api::core::v1::Pod,
    serde::{de::DeserializeOwned, Serialize},
    NamespaceResourceScope,
};
use kube::{
    api::{ListParams, Patch, PatchParams},
    runtime::{conditions::is_pod_running, wait::await_condition},
    Api, Client, Resource,
};
use serde::Deserialize;
use serde_with::DisplayFromStr;
use std::{cmp::max, convert::Infallible, net::SocketAddr, pin::Pin, task::Poll, time::Duration};
use tokio::time::sleep;

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

#[derive(Deserialize, Serialize)]
pub struct PostTimeout {
    #[serde(with = "humantime_serde")]
    pub timeout: Option<Duration>,
}

pub const CN: &str = "namespace-watchdog";

pub async fn own(
    client: Client,
    namespace: String,
    mut watch_args: WatchArgs,
) -> Result<Infallible, anyhow::Error> {
    watch_args.listen = Some("0.0.0.0:8080".parse().unwrap());
    apply(&client, &namespace, resources::service_account(&namespace)).await?;
    apply(&client, &namespace, resources::role(&namespace)).await?;
    apply(&client, &namespace, resources::role_binding(&namespace)).await?;
    apply(
        &client,
        &namespace,
        resources::deployment(&namespace, &watch_args),
    )
    .await?;
    let mut sender = None;
    loop {
        let sender = match sender.is_some() {
            true => sender.as_mut().unwrap(),
            false => {
                let pods = Api::<Pod>::namespaced(client.clone(), &namespace);
                let pod = pods
                    .list(&ListParams::default().limit(1).labels(&format!("name={CN}")))
                    .await?;
                anyhow::ensure!(
                    matches!(pod.metadata.remaining_item_count, Some(0) | None),
                    "More than one possible pod"
                );
                let pod = pod.items.get(0).context("Watchdog pod not found")?;
                let pod = pod
                    .meta()
                    .name
                    .as_ref()
                    .expect("Running pods should have names");
                let running = await_condition(pods.clone(), pod, is_pod_running());
                let _ = tokio::time::timeout(
                    max(watch_args.initial_timeout, Duration::from_secs(90)),
                    running,
                )
                .await?;
                let mut pf = pods.portforward(pod, &[8080]).await?;
                let port = pf.take_stream(8080).unwrap();
                let (snd, connection) =
                    hyper::client::conn::http1::handshake(HyperAdapter(port)).await?;
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        eprintln!("Error in connection: {}", e);
                    }
                });
                sender.insert(snd)
            }
        };

        let http_req = Request::builder()
            .uri("/?timeout=90sec")
            .header("Connection", "keep-alive")
            .method("POST")
            .body(String::new())
            .unwrap();

        let (parts, _body) = sender.send_request(http_req).await?.into_parts();
        ensure!(parts.status == 200);
        sleep(Duration::from_secs(10).into()).await;
    }
}

async fn apply<K>(client: &Client, namespace: &str, resource: K) -> anyhow::Result<()>
where
    K: std::fmt::Debug,
    K: DeserializeOwned,
    K: Serialize,
    K: Clone,
    K: Resource<Scope = NamespaceResourceScope>,
    <K as Resource>::DynamicType: Default,
{
    let kind = K::kind(&default()).to_string();
    let name = &resource
        .meta()
        .name
        .clone()
        .expect(&format!("All resource hava a name, {kind} does not"));
    let id = format!("{kind}/{name}");
    Api::<K>::namespaced(client.clone(), namespace)
        .patch(name, &PatchParams::apply(CN), &Patch::Apply(resource))
        .await
        .with_context(|| format!("Failed to apply resource {id}"))?;
    anyhow::Ok(())
}

pub mod resources {
    use crate::{default, WatchArgs, CN};
    use humantime_serde::re::humantime;
    use k8s_openapi::{
        api::{
            apps::v1::{Deployment, DeploymentSpec, DeploymentStrategy},
            core::v1::{
                Container, Namespace, PodSpec, PodTemplateSpec, ResourceRequirements,
                ServiceAccount,
            },
            rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject},
        },
        apimachinery::pkg::{api::resource::Quantity, apis::meta::v1::LabelSelector},
    };
    use kube::{core::ObjectMeta, Resource, ResourceExt as _};
    use std::collections::BTreeMap;

    pub fn role_binding(namespace: &str) -> RoleBinding {
        RoleBinding {
            metadata: metadata(&namespace),
            role_ref: RoleRef {
                api_group: "rbac.authorization.k8s.io".into(),
                kind: Role::kind(&()).into(),
                name: role(namespace).name_unchecked(),
            },
            subjects: Some(vec![Subject {
                kind: ServiceAccount::kind(&()).into(),
                name: service_account(&namespace).name_unchecked(),
                namespace: ss(namespace),
                ..default()
            }]),
        }
    }

    pub fn role(namespace: &str) -> Role {
        Role {
            metadata: metadata(namespace),
            rules: Some(vec![PolicyRule {
                api_groups: Some(vec!["".into()]),
                resources: Some(vec!["namespaces".into()]),
                verbs: vec!["delete".into()],
                ..default()
            }]),
        }
    }

    pub fn service_account(namespace: &str) -> ServiceAccount {
        ServiceAccount {
            metadata: metadata(namespace),
            ..default()
        }
    }

    pub fn deployment(namespace: &str, watch_args: &WatchArgs) -> Deployment {
        let labels = Some(BTreeMap::from([("name".to_string(), CN.to_string())]));
        let resources = Some(BTreeMap::from([
            ("cpu".to_string(), Quantity("5m".into())),
            ("memory".to_string(), Quantity("10M".into())),
        ]));
        #[rustfmt::skip]
    let args = {
        let WatchArgs {
            max_timeout,
            initial_timeout,
            sigusr1_timeout,
            listen,
        } = watch_args;
        [
            Some(format!("--initial-timeout={}", humantime::Duration::from(*initial_timeout))),
            max_timeout.map(humantime::Duration::from).map(|to| format!("--max-timeout={to}")),
            sigusr1_timeout.map(humantime::Duration::from).map(|to| format!("--sigusr1-timeout={to}")),
            listen.map(|l| format!("--listen={l}")),
        ] 
        .into_iter()
        .filter_map(|v| v)
        .collect::<Vec<_>>()
    };
        Deployment {
            metadata: metadata(namespace),
            spec: Some(DeploymentSpec {
                replicas: Some(1),
                strategy: Some(DeploymentStrategy {
                    type_: ss("Recreate"),
                    ..default()
                }),
                selector: LabelSelector {
                    match_labels: labels.clone(),
                    ..default()
                },
                template: PodTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels,
                        ..default()
                    }),
                    spec: Some(PodSpec {
                        service_account_name: ss(CN),
                        containers: vec![Container {
                            name: "main".into(),
                            args: Some(args),
                            image: ss(format!("liftm/kubernetes-{CN}")),
                            image_pull_policy: ss("Always"),
                            resources: Some(ResourceRequirements {
                                limits: resources.clone(),
                                requests: resources,
                                ..default()
                            }),
                            ..default()
                        }],
                        ..default()
                    }),
                },
                ..default()
            }),
            ..default()
        }
    }

    pub fn namespace(namespace: &str) -> Namespace {
        Namespace {
            metadata: ObjectMeta {
                name: ss(namespace),
                ..default()
            },
            ..default()
        }
    }

    fn metadata(namespace: &str) -> ObjectMeta {
        ObjectMeta {
            name: ss(CN),
            namespace: ss(namespace),
            ..default()
        }
    }

    fn ss(s: impl Into<String>) -> Option<String> {
        Some(s.into())
    }
}

pub(crate) fn default<T: Default>() -> T {
    std::default::Default::default()
}

struct HyperAdapter<R>(R);
impl<R: tokio::io::AsyncRead + Unpin> hyper::rt::Read for HyperAdapter<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        use tokio::io::ReadBuf;
        let mut tbuf = ReadBuf::uninit(unsafe { buf.as_mut() });
        match Pin::new(&mut self.0).poll_read(cx, &mut tbuf) {
            Poll::Ready(res) => {
                let advanced = tbuf.filled().len();
                unsafe { buf.advance(advanced) };
                Poll::Ready(res)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<R: tokio::io::AsyncWrite + Unpin> hyper::rt::Write for HyperAdapter<R> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
