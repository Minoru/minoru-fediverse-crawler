FROM debian:12

ENV DEBIAN_FRONTEND noninteractive

RUN apt-get update \
    && apt-get install --assume-yes --no-install-recommends \
        build-essential libsqlite3-dev ca-certificates wget \
    && apt-get autoremove \
    && apt-get clean

RUN addgroup --gid 1000 builder \
    && adduser --home /home/builder --uid 1000 --ingroup builder \
        --disabled-password --shell /bin/bash builder \
    && mkdir -p /home/builder/src \
    && chown -R builder:builder /home/builder

USER builder
ENV HOME /home/builder
WORKDIR /home/builder/src

RUN wget -O $HOME/rustup.sh --secure-protocol=TLSv1_3 https://sh.rustup.rs \
    && chmod +x $HOME/rustup.sh \
    && $HOME/rustup.sh -y \
        --default-host x86_64-unknown-linux-gnu \
        --default-toolchain 1.72.1 \
    && chmod a+w $HOME/.cargo

ENV PATH $HOME/.cargo/bin:$PATH
RUN mkdir -p $HOME/.cargo/registry
