FROM ubuntu:26.04

RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
    ca-certificates \
    git \
    openssh-client \
  && rm -rf /var/lib/apt/lists/*

ARG TARGETARCH
COPY ${TARGETARCH}/whim /usr/local/bin/whim

WORKDIR /app
ENTRYPOINT ["/usr/local/bin/whim"]
