SHELL := /bin/bash

RUST_VERSION := 1.90
DOCKER := DOCKER_BUILDKIT=1 docker

.PHONY: help ci-local ci-local-deep build test lint fmt lock install-hooks e2e-up

help:
	@echo "  make ci-local   run every gate (the pre-push gate, and what CI mirrors)"
	@echo "  make build      compile the binary"
	@echo "  make test       the unit tests: every check against a fake machine"
	@echo "  make lint       rustfmt --check and clippy with warnings denied"
	@echo "  make fmt        apply rustfmt"
	@echo "  make lock       regenerate Cargo.lock"
	@echo "  make e2e-up     install into a throwaway namespace and answer the wizard"

# Local green is the completion signal; CI is confirmation.
ci-local: build test lint
	@echo
	@echo "ci-local: GREEN"

ci-local-deep: ci-local

build:
	@$(DOCKER) build -f Dockerfile.rust --target check . >/dev/null 2>&1 \
		|| { echo "build FAILED; see it with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target check ." >&2; exit 1; }
	@echo "build OK: the binary compiles"

test:
	@$(DOCKER) build -f Dockerfile.rust --target test . >/dev/null 2>&1 \
		|| { echo "test FAILED; see the output with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target test --progress=plain ." >&2; exit 1; }
	@echo "test OK: the checks, the params file's refusals and the argument parser"

lint:
	@$(DOCKER) build -f Dockerfile.rust --target lint . >/dev/null 2>&1 \
		|| { echo "lint FAILED; see it with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target lint --progress=plain ." >&2; exit 1; }
	@echo "lint OK: formatting and clippy clean"

# Applied in a container and written back, because the host has no toolchain.
fmt:
	@$(DOCKER) run --rm -v "$(CURDIR)":/w -w /w rust:$(RUST_VERSION)-slim-bookworm \
		sh -c 'rustup component add rustfmt >/dev/null 2>&1; cargo fmt --all'
	@echo "fmt: applied"

lock:
	@$(DOCKER) run --rm -v "$(CURDIR)":/w -w /w rust:$(RUST_VERSION)-slim-bookworm \
		sh -c 'apt-get update >/dev/null && apt-get install -y --no-install-recommends git >/dev/null && cargo generate-lockfile'
	@echo "lock: Cargo.lock regenerated"

# A real deployment, driven by this binary rather than by a test's HTTP calls:
# the same path `make e2e-first-run` proves in meridian-core. Not yet written;
# the target exists so that adding it is a change to one line.
e2e-up:
	@echo "e2e-up: not built yet. spec/the-cli says what it must prove." >&2
	@exit 1

install-hooks:
	@git config core.hooksPath hooks
	@echo "hooks installed: git push now runs 'make ci-local' first"
