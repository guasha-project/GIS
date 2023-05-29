#!/opt/bin/bash

# GIS upgrade script for Keenetic routers with Entware

json=$(curl -s "https://api.github.com/repos/guasha-project/GIS/releases/latest")
upstreamver=$(echo "$json" | jq -r ".tag_name")

curver=$(gis -v | cut -c7-25)

changed=$(diff <(echo "$curver") <(echo "$upstreamver"))

if [ "$changed" != "" ]
then
  echo "Upgrading from $curver to $upstreamver"
  /opt/etc/init.d/S98gis stop
  wget https://github.com/guasha-project/GIS/releases/download/$upstreamver/gis-linux-mipsel-$upstreamver-nogui -O /opt/bin/gis
  chmod +x /opt/bin/gis
  /opt/etc/init.d/S98gis start
else
  echo "No need to upgrade, $curver is the current version"
fi
