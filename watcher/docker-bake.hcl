variable "watchdog_image_tag" {
  default = "latest"
}

target "default" {
  platforms = ["linux/amd64", "linux/arm64"]
  tags = ["docker.io/liftm/kubernetes-namespace-watchdog:${watchdog_image_tag}"]
  context = ".."
  dockerfile = "watcher/Dockerfile"
}
