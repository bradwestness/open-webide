FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends openssh-server git && rm -rf /var/lib/apt/lists/*
COPY server.sh /server.sh
ENTRYPOINT ["bash", "/server.sh"]
