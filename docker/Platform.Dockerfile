FROM rust:1.98.1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa AS build
ENV CARGO_BUILD_JOBS=1
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY proto ./proto
COPY spec ./spec
RUN mkdir -p /out
RUN --mount=type=cache,id=rx-platform-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rx-platform-release,target=/src/target \
    cargo build --release --locked -p rx-platformd && cp target/release/rx-platformd target/release/rx-package-store /out/

FROM ubuntu:24.04@sha256:224a1869083a311ef3f13648a154ba79832fbef6364d31493642ca03082da254 AS runtime
LABEL org.opencontainers.image.title="RX platform runtime draft" \
      org.opencontainers.image.description="ROS-independent platform services; qualification authority is not connected"
RUN mkdir -p /etc/rx/platform /var/lib/rx /run/rx && chown 10001:10001 /var/lib/rx /run/rx
COPY --from=build /out/rx-platformd /out/rx-package-store /usr/local/bin/
USER 10001:10001
WORKDIR /var/lib/rx
EXPOSE 7443 8443
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/rx-platformd"]
CMD ["run", "/etc/rx/platform/startup.json"]
