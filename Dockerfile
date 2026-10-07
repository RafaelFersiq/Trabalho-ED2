# Multi-stage build para gerar o binário estático /engine em Linux amd64
FROM rust:1-slim as builder

WORKDIR /usr/src/engine
COPY Cargo.toml ./
# Cria dummy main para cache de dependências
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release || true

# Copia código-fonte real
COPY src ./src
# Força recompilação com o código real
RUN touch src/main.rs && cargo build --release

# Imagem final de execução
FROM debian:bookworm-slim

# Copia o binário compilado para /engine conforme especificação
COPY --from=builder /usr/src/engine/target/release/engine /engine

ENTRYPOINT ["/engine"]
