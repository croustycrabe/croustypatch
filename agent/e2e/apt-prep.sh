#!/bin/bash
# Runs inside ubuntu:24.04: three demo packages, each in a different "restore point" situation after an update.
set -u
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null 2>&1; apt-get install -y -qq curl ca-certificates dpkg-dev apt-utils jq util-linux >/dev/null 2>&1
mkdir -p /repo /tmp/build
mkpkg() { # mkpkg NAME VERSION
  local d=/tmp/build/$1_$2; rm -rf $d; mkdir -p $d/DEBIAN $d/usr/share/$1
  printf 'Package: %s\nVersion: %s\nArchitecture: all\nMaintainer: test <t@example.org>\nDescription: demo package\n' "$1" "$2" > $d/DEBIAN/control
  echo "$2" > $d/usr/share/$1/version
  dpkg-deb --build -Zgzip $d /repo/$1_$2_all.deb >/dev/null
}
reindex() { (cd /repo && apt-ftparchive packages . > Packages && gzip -kf Packages); }
for p in cdemo-a cdemo-b cdemo-c; do mkpkg $p 1.0; done
reindex
echo 'deb [trusted=yes] file:/repo ./' > /etc/apt/sources.list.d/repo.list
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq cdemo-a cdemo-b cdemo-c >/dev/null 2>&1
# 1.1 is published for all three
for p in cdemo-a cdemo-b cdemo-c; do mkpkg $p 1.1; done
# a: old .deb survives only in apt's cache      b: old version stays listed in the repository      c: gone from both
cp /repo/cdemo-a_1.0_all.deb /var/cache/apt/archives/
rm -f /repo/cdemo-a_1.0_all.deb /repo/cdemo-c_1.0_all.deb
reindex
apt-get update -qq >/dev/null 2>&1
echo "installed:  $(dpkg-query -W -f='${Package}=${Version} ' cdemo-a cdemo-b cdemo-c)"
for p in cdemo-a cdemo-b cdemo-c; do echo "madison $p: $(apt-cache madison $p | awk -F'|' '{printf "%s ", $2}')"; done
echo "apt cache:  $(ls /var/cache/apt/archives | grep cdemo | tr '\n' ' ')"
