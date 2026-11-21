# Stage 1: Build the frontend
FROM node:22-slim AS frontend-build

WORKDIR /frontend

COPY frontend/package*.json ./
RUN npm ci

COPY frontend/ ./
RUN npm run build

################################################################################
# Stage 2: Build the Rust application
FROM docker.io/rust:1.94.1-slim-bookworm AS build

## cargo package name: customize here or provide via --build-arg
ARG pkg=docs-app

WORKDIR /build

COPY . .


RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*

## for Enterprise CAs
## RUN apt-get install -y ca-certificates
## RUN cp ./MyRootCA.crt /usr/local/share/ca-certificates/
## RUN update-ca-certificates

RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    set -eux; \
    cargo build --release --locked; \
    objcopy --compress-debug-sections target/release/$pkg ./main

################################################################################

FROM docker.io/debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc

WORKDIR /app

RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

## copy the main binary
COPY --from=build /build/main .

## copy runtime assets which may or may not exist
COPY --from=frontend-build frontend/dist ./frontend/dist
COPY --from=build /build/Rocket.toml ./Rocket.toml
##  COPY --from=build /build/stati[c] ./static
##  COPY --from=build /build/template[s] ./templates


## ensure the container listens globally on port 8080
ENV ROCKET_ADDRESS=0.0.0.0
ENV ROCKET_PORT=8080

ENTRYPOINT ["./main"]
