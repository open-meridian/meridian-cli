# The plugin's image. It runs beside a sidecar in the deployment's cluster, and
# reaches nothing but that sidecar.
FROM python:3.12-slim
ENV PIP_DISABLE_PIP_VERSION_CHECK=1 PIP_NO_CACHE_DIR=1
WORKDIR /plugin
COPY . .
# PIP_FIND_LINKS, if given, is a directory of wheels to prefer -- an SDK built
# locally rather than taken from PyPI. Unset, everything comes from PyPI.
ARG PIP_FIND_LINKS
RUN pip install .
USER 65532
CMD ["reference-plugin"]
