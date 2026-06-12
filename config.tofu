terraform {
  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.45"
    }
  }
}

provider "hcloud" {} # reads HCLOUD_TOKEN from env

resource "hcloud_ssh_key" "me" {
  name       = "vitor"
  public_key = file("~/.ssh/id_ed25519.pub")
}

resource "hcloud_firewall" "chat" {
  name = "chat-rs"

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "3000" # gRPC signaling
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  rule {
    direction  = "in"
    protocol   = "udp"
    port       = "50000" # WebRTC media (matches UDP_PORT)
    source_ips = ["0.0.0.0/0", "::/0"]
  }
}

resource "hcloud_server" "chat" {
  name         = "chat-rs"
  server_type  = "cpx11"
  location     = "hil" # Hillsboro, Oregon
  image        = "debian-12"
  ssh_keys     = [hcloud_ssh_key.me.id]
  firewall_ids = [hcloud_firewall.chat.id]

  public_net {
    ipv4_enabled = true
    ipv6_enabled = true
  }

  user_data = <<-EOT
    #cloud-config
    package_update: true
    packages: [ca-certificates, curl]
    runcmd:
      - curl -fsSL https://get.docker.com | sh
  EOT
}

output "public_ip" {
  value = hcloud_server.chat.ipv4_address
}
