use std::collections::BTreeMap;

use anyhow::Context;
use k8s_openapi::{
    api::{
        apps::v1::{Deployment, DeploymentSpec, DeploymentStrategy},
        core::v1::{Container, PodSpec, PodTemplateSpec, ResourceRequirements, ServiceAccount},
        rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject},
    },
    apimachinery::pkg::{api::resource::Quantity, apis::meta::v1::LabelSelector},
    serde::{de::DeserializeOwned, Serialize},
    NamespaceResourceScope,
};
use kube::{
    api::{Patch, PatchParams},
    core::ObjectMeta,
    Api, Client, Resource, ResourceExt as _,
};
use kubernetes_namespace_watchdog_lib::WatchArgs;

fn default<T: Default>() -> T {
    std::default::Default::default()
}

fn ss(s: impl Into<String>) -> Option<String> {
    Some(s.into())
}

#[derive(clap::Parser)]
struct DeployArgs {
    #[arg(short, long)]
    // namespace to watch
    namespace: String,
    #[command(flatten)]
    watch_args: WatchArgs,
}

const CN: &str = "namespace-watchdog";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let DeployArgs {
        namespace,
        watch_args,
    } = clap::Parser::parse();
    let client = Client::try_default()
        .await
        .context("Failed to construct client")?;
    apply(&client, &namespace, service_account(&namespace)).await?;
    apply(&client, &namespace, role(&namespace)).await?;
    apply(&client, &namespace, role_binding(&namespace)).await?;
    apply(&client, &namespace, deployment(&namespace, watch_args)).await?;
    Ok(())
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

fn role_binding(namespace: &str) -> RoleBinding {
    RoleBinding {
        metadata: cnmeta(&namespace),
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

fn role(namespace: &str) -> Role {
    Role {
        metadata: cnmeta(namespace),
        rules: Some(vec![PolicyRule {
            api_groups: Some(vec!["".into()]),
            resources: Some(vec!["namespaces".into()]),
            verbs: vec!["delete".into()],
            ..default()
        }]),
    }
}

fn service_account(namespace: &str) -> ServiceAccount {
    ServiceAccount {
        metadata: cnmeta(namespace),
        ..default()
    }
}

fn deployment(namespace: &str, watch_args: WatchArgs) -> Deployment {
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
            Some(format!("--initial-timeout={}", humantime::Duration::from(initial_timeout))),
            max_timeout.map(humantime::Duration::from).map(|to| format!("--max-timeout={to}")),
            sigusr1_timeout.map(humantime::Duration::from).map(|to| format!("--sigusr1-timeout={to}")),
            listen.map(|l| format!("--listen={l}")),
        ] 
        .into_iter()
        .filter_map(|v| v)
        .collect::<Vec<_>>()
    };
    Deployment {
        metadata: cnmeta(namespace),
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
                        #[cfg(feature = "fixed-image-hash")]
                        image: ss(format!("liftm/{CN}@sha256")),
                        #[cfg(not(feature = "fixed-image-hash"))]
                        image: ss(format!("liftm/{CN}")),
                        image_pull_policy: ss(match cfg!(feature = "fixed-image-hash") {
                            true => "IfNotPresent",
                            false => "Always",
                        }),
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

fn cnmeta(namespace: &str) -> ObjectMeta {
    ObjectMeta {
        name: ss(CN),
        namespace: ss(namespace),
        ..default()
    }
}
