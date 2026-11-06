# Stage 0 runner. Built by compose.example.yaml from a pinned commit of this repo.
# Build before the model loads: the server has little spare memory and no swap.
FROM rust:1.97-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
ENV CARGO_BUILD_JOBS=1
RUN cargo build --release --locked --bin stage0-runner

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 runner && mkdir /out && chown runner /out
COPY --from=build /src/target/release/stage0-runner /usr/local/bin/stage0-runner
USER runner
ENV STAGE0_OUT=/out STAGE0_JOB=/tmp/job
ENTRYPOINT ["/usr/local/bin/stage0-runner"]
