# The container image ghcr.io/thkprado/jlens: the static Linux binary of a release archive, and
# nothing else (no shell, no libc). The build context is a directory holding the archives of the
# x86-64 and ARM64 musl targets (jlens-<name>-<target>.tar.gz); BuildKit's TARGETARCH picks the
# one of the platform being built, so a multi-platform build runs no emulated step. The Release
# workflow builds it from the release archives, and CI from its own:
#
#   docker build -f crates/jlens-cli/Dockerfile -t jlens dist

ARG TARGETARCH

# ADD unpacks a local tar archive, here into /jlens-<name>-<target>/.
FROM scratch AS archive-amd64
ADD jlens-*-x86_64-unknown-linux-musl.tar.gz /

FROM scratch AS archive-arm64
ADD jlens-*-aarch64-unknown-linux-musl.tar.gz /

FROM archive-${TARGETARCH} AS archive

FROM scratch
COPY --from=archive /jlens-*/jlens /jlens-*/LICENSE /
LABEL org.opencontainers.image.title="jlens" \
      org.opencontainers.image.description="The jlens command line: a JSONata engine with evaluation audits" \
      org.opencontainers.image.source="https://github.com/thkprado/jlens" \
      org.opencontainers.image.documentation="https://thkprado.github.io/jlens/guides/cli.html" \
      org.opencontainers.image.licenses="MIT"
# Not root. Mount the files to read on /work (docker run -v "$PWD:/work" ...).
USER 65532:65532
WORKDIR /work
ENTRYPOINT ["/jlens"]
