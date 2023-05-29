FROM debian:stretch-slim

LABEL Description="Guasha Free Identity System"
LABEL URL="https://github.com/guasha-project/GIS/releases"

ARG arch=amd64
ARG srv_port=46866
ARG dns_port=53

RUN apt-get update -y && \
    apt-get install -y curl && \
    curl -SsL "https://github.com/guasha-project/GIS/releases/download/$(curl --silent "https://api.github.com/repos/guasha-project/GIS/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')/gis-${arch}-$(curl --silent "https://api.github.com/repos/guasha-project/GIS/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')-nogui.deb" -o /tmp/gis.deb  && \
    dpkg -i /tmp/gis.deb && \
    apt-get purge -y curl && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /tmp/* && \
    rm -rf /var/lib/{apt,dpkg,cache,log}/

EXPOSE ${srv_port}
EXPOSE ${dns_port}
EXPOSE ${dns_port}/udp

WORKDIR /var/lib/gis

CMD ["/usr/bin/gis", "-n", "-c", "/etc/gis.conf"]
