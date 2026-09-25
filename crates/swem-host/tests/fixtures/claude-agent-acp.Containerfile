FROM docker.io/library/node@sha256:0d130e2ee18e88e1561375276daced6bff032539200173f2daf48c2e33f38ff5

WORKDIR /opt/swem/agent

# The ACP adapter is the published upstream distribution. Network is needed
# only while this image is built; active SWEM leases use --pull=never and a
# resolved local image ID. The committed lock freezes the complete npm tree;
# npm ci fails instead of rewriting it.
COPY package.json package-lock.json ./
RUN npm ci --ignore-scripts --omit=dev --no-audit --no-fund \
    && npm cache clean --force \
    && node -e "const p=require('./node_modules/@agentclientprotocol/claude-agent-acp/package.json'); if(p.version!=='0.70.0') process.exit(1)" \
    && test -f node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js \
    && test -x node_modules/@anthropic-ai/claude-agent-sdk-linux-x64/claude

USER 1000:1000
