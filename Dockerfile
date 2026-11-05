# The simpled controller image, ghcr.io/vkubiv/simpled:<version>
# (docs/design/controller.md). The same simpled binary as the release tarballs,
# built static against musl, on an empty base.
#
# The build context holds only the binaries: simpled_linux_amd64 and
# simpled_linux_arm64. Nothing runs during the build, so a multi-platform
# build needs no emulation:
#
#   docker buildx build --platform linux/amd64,linux/arm64 -t <tag> dist/

# Only the CA bundle comes from here, for the controller's outbound HTTPS (ACME).
FROM --platform=$BUILDPLATFORM alpine:3.20 AS certs

FROM scratch
ARG TARGETARCH
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY simpled_linux_${TARGETARCH} /simpled
# Not root. The generated stack adds the group that owns the Docker socket.
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/simpled", "controller"]
CMD ["--config", "/etc/simpled/controller.json"]
