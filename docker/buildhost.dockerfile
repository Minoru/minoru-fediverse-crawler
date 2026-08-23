FROM debian:12

ENV DEBIAN_FRONTEND noninteractive

RUN apt-get update \
    && apt-get install --assume-yes --no-install-recommends \
        build-essential libsqlite3-dev ca-certificates wget \
    && apt-get autoremove \
    && apt-get clean

RUN wget -O /tmp/rustup.sh --secure-protocol=TLSv1_3 https://sh.rustup.rs \
    && chmod +x /tmp/rustup.sh \
    && /tmp/rustup.sh -y \
        --default-host x86_64-unknown-linux-gnu \
        --default-toolchain 1.98.0 \
    && chmod a+w /root/.cargo

ENV PATH /root/.cargo/bin:$PATH
RUN mkdir -p /root/.cargo/registry

WORKDIR /src
