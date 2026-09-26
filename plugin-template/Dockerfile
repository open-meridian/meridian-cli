# The plugin's image. It runs beside a sidecar in the deployment's cluster, and
# reaches nothing but that sidecar.
#
# Built on the base image for the SDK this plugin pins: Python and
# open-meridian, the same version as pyproject.toml's, already installed. So
# the image's own layers are this plugin and whatever it needs besides, and an
# upload sends only those. Move the two together when moving to a new SDK.
#
# BASE is for building against a base made locally rather than a released one.
ARG BASE=ghcr.io/open-meridian/plugin-python:0.2.0
FROM ${BASE}
WORKDIR /plugin
COPY . .
RUN pip install .
USER 65532
CMD ["reference-plugin"]
