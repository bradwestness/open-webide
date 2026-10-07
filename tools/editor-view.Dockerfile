FROM ghcr.io/spinframework/spin:v4.1.0

# Match Chromium and its driver through the same distribution packages. This
# image measures built WASM artifacts; it never creates another Cargo target.
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium chromium-driver fonts-dejavu-core fonts-noto-cjk \
    fonts-noto-color-emoji git procps python3 \
    && rm -rf /var/lib/apt/lists/*

ENV CI=true CHROME=/usr/bin/chromium CHROMEDRIVER=/usr/bin/chromedriver
WORKDIR /workspace/repos/openwebide
ENTRYPOINT ["python3", "tools/measure-editor-view.py"]
