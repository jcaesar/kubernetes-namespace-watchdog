use anyhow::Context;
use k8s_openapi::api::core::v1::Namespace;
use kube::{
    api::{Patch, PatchParams},
    Api, Client,
};
use namespace_watchdog_owner::{resources, CN};

#[derive(clap::Parser)]
struct DeployArgs {
    /// namespace to watch
    #[arg(short, long)]
    namespace: String,
    /// Create the namespace if it doesn't exist
    #[arg(short, long, value_enum, default_value = "auto")]
    create: CreateNamespace,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum CreateNamespace {
    Never,
    Auto,
    Always,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let DeployArgs { namespace, create } = clap::Parser::parse();
    let client = Client::try_default()
        .await
        .context("Failed to construct client")?;
    let ns_api = Api::<Namespace>::all(client.clone());
    let nsr = resources::namespace(&namespace);
    match create {
        CreateNamespace::Never => Ok(()),
        CreateNamespace::Auto => ns_api
            .patch(&namespace, &PatchParams::apply(CN), &Patch::Apply(nsr))
            .await
            .map(|_| ()),
        CreateNamespace::Always => ns_api.create(&Default::default(), &nsr).await.map(|_| ()),
    }
    .context("Create namespace")?;
    namespace_watchdog_owner::own(client, namespace).await?;
    Ok(())
}
