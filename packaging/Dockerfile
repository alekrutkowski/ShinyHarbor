# Multi-architecture source build. Pin base-image digests and a reviewed Cargo.lock for production.
FROM rust:slim-bookworm AS builder
WORKDIR /build
COPY . .
RUN if [ ! -f Cargo.lock ]; then cargo generate-lockfile; fi \
 && cargo build --locked --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends r-base r-base-dev ca-certificates libssl-dev libcurl4-openssl-dev \
 && Rscript -e 'install.packages("shiny", repos="https://cloud.r-project.org", Ncpus=2)' \
 && useradd --system --create-home --uid 10001 harbor \
 && mkdir -p /apps /var/lib/shinyharbor /etc/shinyharbor \
 && chown -R harbor:harbor /var/lib/shinyharbor \
 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/shinyharbor /usr/local/bin/shinyharbor
COPY --from=builder /build/Cargo.lock /usr/share/shinyharbor/Cargo.lock
COPY LICENSE ATTRIBUTIONS.md /usr/share/shinyharbor/
USER harbor
WORKDIR /apps
EXPOSE 3838 443
ENTRYPOINT ["shinyharbor"]
CMD ["serve", "--config", "/etc/shinyharbor/harbor.toml"]
