#!/usr/bin/env python3
"""Rebuild ../brand-icons.json from the Simple Icons package (https://simpleicons.org, CC0 1.0).

    npm pack simple-icons && mkdir x && tar -xzf simple-icons-*.tgz -C x
    python3 generate.py x/package

The file holds, per icon, its title, brand colour and SVG path (24x24 viewBox). The server turns an icon the catalog
names as `brand:<slug>` into an image of that colour; nothing is fetched from the internet at run time.
"""
import json, re, sys

# Applications an administrator is likely to put in an app catalog or recognise by their logo.
SLUGS = """git github gitlab gitea python nodedotjs deno bun go rust openjdk php ruby perl lua cplusplus c dotnet gnubash zsh fish
neovim vim emacs sublimetext jetbrains intellijidea pycharm androidstudio visualstudio postman insomnia
docker podman kubernetes helm ansible terraform vagrant packer
sqlite postgresql mariadb mysql redis mongodb influxdb dbeaver
htop tmux gnu wireshark wireguard openvpn tailscale nmap
borgbackup keepassxc bitwarden 1password veracrypt syncthing nextcloud
firefox googlechrome brave vivaldi operagx tor thunderbird protonmail
libreoffice onlyoffice gimp inkscape krita blender darktable obsstudio kdenlive audacity vlcmediaplayer handbrake spotify
filezilla qbittorrent transmission
discord telegram signal element slack zoom teams
steam lutris wine
ubuntu debian archlinux fedora linuxmint opensuse alpinelinux nixos linux proxmox truenas
nginx apache traefikproxy caddy haproxy grafana prometheus elastic kibana zabbix
home-assistant homeassistant pihole adguard jellyfin plex immich vaultwarden portainer
kde gnome xfce zedindustries
libreoffice vscodium inkscape ffmpeg imagemagick""".split()

def main(pkg):
    data = {d["slug"]: d for d in json.load(open(f"{pkg}/data/simple-icons.json"))}
    out, missing = {}, []
    for slug in sorted(set(SLUGS)):
        d = data.get(slug)
        if not d:
            missing.append(slug)
            continue
        svg = open(f"{pkg}/icons/{slug}.svg", encoding="utf-8").read()
        m = re.search(r'<path d="([^"]+)"', svg)
        if not m:
            missing.append(slug)
            continue
        out[slug] = {"title": d["title"], "hex": d["hex"], "path": m.group(1)}
    json.dump(out, open(__file__.rsplit("/", 1)[0] + "/../brand-icons.json", "w", encoding="utf-8"), ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    print(f"{len(out)} icons written; not in Simple Icons: {' '.join(missing)}")

main(sys.argv[1])
