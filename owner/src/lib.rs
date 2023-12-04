use anyhow::{ensure, Context};
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
use std::{convert::Infallible, pin::Pin, task::Poll, time::Duration};
use tokio::time::sleep;

/// Name of kubernetes resources being created
pub const CN: &str = "namespace-watchdog";

/// Install resources and keep polling watcher
pub async fn own(client: Client, namespace: String) -> Result<Infallible, anyhow::Error> {
    apply(&client, &namespace, resources::service_account(&namespace)).await?;
    apply(&client, &namespace, resources::role(&namespace)).await?;
    apply(&client, &namespace, resources::role_binding(&namespace)).await?;
    apply(&client, &namespace, resources::deployment(&namespace)).await?;

    let mut failures = 0;
    let mut error = None;
    while failures < 3 {
        let mut succeeded_once = false;
        let res = keep_alive(&client, &namespace, &mut succeeded_once).await;
        match succeeded_once {
            true => {
                failures = 0;
                error.take();
            }
            false => {
                failures += 1;
                error.get_or_insert(res.unwrap_err());
            }
        }
        sleep(Duration::from_secs(10).into()).await;
    }
    Err(error.unwrap())
}

async fn keep_alive(
    client: &Client,
    namespace: &str,
    succeeded_once: &mut bool,
) -> Result<Infallible, anyhow::Error> {
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
    let _ = tokio::time::timeout(Duration::from_secs(90), running).await?;
    let mut pf = pods.portforward(pod, &[8080]).await?;
    let port = pf.take_stream(8080).unwrap();
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(HyperAdapter(port)).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Error in connection: {}", e);
        }
    });
    loop {
        let http_req = Request::builder()
            .uri("/?timeout=90sec")
            .header("Connection", "keep-alive")
            .method("POST")
            .body(String::new())
            .unwrap();

        let (parts, _body) = sender.send_request(http_req).await?.into_parts();
        ensure!(parts.status == 200);
        *succeeded_once = true;
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
    use crate::{default, CN};
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

    pub fn deployment(namespace: &str) -> Deployment {
        let labels = Some(BTreeMap::from([("name".to_string(), CN.to_string())]));
        let resources = Some(BTreeMap::from([
            ("cpu".to_string(), Quantity("5m".into())),
            ("memory".to_string(), Quantity("10M".into())),
        ]));
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
                            args: Some(vec![
                                "--initial-timeout=60 seconds".into(),
                                "--listen=0.0.0.0:8080".into(),
                            ]),
                            image: ss(format!(
                                "liftm/kubernetes-{CN}:{}",
                                env!("CARGO_PKG_VERSION")
                            )),
                            image_pull_policy: ss("IfNotPresent"),
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
