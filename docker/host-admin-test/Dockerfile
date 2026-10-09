FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends openssh-server sudo procps iproute2 util-linux && rm -rf /var/lib/apt/lists/* && \
    useradd --create-home admin && printf 'admin:fixture-password\n' | chpasswd && \
    printf 'admin ALL=(root) ALL\n' > /etc/sudoers.d/admin && chmod 440 /etc/sudoers.d/admin && \
    printf '11223344556677889900112233445566\n' > /etc/machine-id
COPY server.sh /server.sh
ENTRYPOINT ["/bin/sh", "/server.sh"]
