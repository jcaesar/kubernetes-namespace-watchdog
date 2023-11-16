target "default" {
  platforms = ["linux/amd64", "linux/arm64"]
  tags = ["docker.io/liftm/kubernetes-namespace-watchdog:latest"]
  context = ".."
  dockerfile = "watcher/Dockerfile"
}
