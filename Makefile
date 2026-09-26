SHELL := /bin/bash

RUST_VERSION := 1.90
DOCKER := DOCKER_BUILDKIT=1 docker

.PHONY: help ci-local ci-local-deep build test lint fmt lock install-hooks e2e-up \
        vendor-template check-vendored-template

help:
	@echo "  make ci-local   run every gate (the pre-push gate, and what CI mirrors)"
	@echo "  make build      compile the binary"
	@echo "  make test       the unit tests: every check against a fake machine"
	@echo "  make lint       rustfmt --check and clippy with warnings denied"
	@echo "  make fmt        apply rustfmt"
	@echo "  make lock       regenerate Cargo.lock"
	@echo "  make e2e-up     install into a throwaway namespace and answer the wizard"
	@echo "  make vendor-template  move the scaffold \`plugin new\` writes to SDK_REV"

# Local green is the completion signal; CI is confirmation.
ci-local: check-vendored-template build test lint
	@echo
	@echo "ci-local: GREEN"

ci-local-deep: ci-local

# What `meridian plugin new` writes: meridian-python's template/, at one SDK
# revision, copied here and compiled into the binary, so scaffolding works
# offline and gives the same plugin every time. The template lives beside the
# SDK it is written against and is tested there against the real sidecar;
# this copy is held to it the way the SDK's bindings are held to the schema.
SDK_REV  := 8a292e930f972d9ee6cc3d6313277c3f8d59f8b1
SDK_REPO := https://github.com/open-meridian/meridian-python.git
SCRATCH  := .sdk-scratch

define fetch_template
	rm -rf $(SCRATCH) && git init -q $(SCRATCH) \
	&& git -C $(SCRATCH) fetch -q --depth 1 $(SDK_REPO) $(SDK_REV) \
	&& git -C $(SCRATCH) checkout -q FETCH_HEAD -- template
endef

vendor-template:
	@$(fetch_template)
	@rm -rf plugin-template && cp -R $(SCRATCH)/template plugin-template && rm -rf $(SCRATCH)
	@echo "vendor-template: plugin-template is meridian-python's template at $(SDK_REV)"

check-vendored-template:
	@$(fetch_template) || { echo "check-vendored-template: could not fetch meridian-python at $(SDK_REV)" >&2; rm -rf $(SCRATCH); exit 1; }
	@if diff -r $(SCRATCH)/template plugin-template >/dev/null; then \
		rm -rf $(SCRATCH); echo "check-vendored-template OK: the scaffold is meridian-python's template at $(SDK_REV)"; \
	else \
		rm -rf $(SCRATCH); echo "check-vendored-template FAILED: plugin-template is not meridian-python's template at $(SDK_REV). Run 'make vendor-template'." >&2; exit 1; \
	fi

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

# A real deployment, driven by this binary rather than by a test's HTTP calls
# (spec/the-cli's verification): meridian-core's cluster run, with
# `meridian up --params` installing the chart and answering the wizard, and
# every check after it -- the database, the administrator's permission, their
# sign-in, and a real browser -- as the run makes them for its own driver.
# Needs meridian-core and meridian-platform beside this checkout, and a
# cluster in the current kube context.
CORE ?= ../meridian-core

e2e-up:
	@test -f "$(CORE)/e2e/cluster/run.py" || { echo "no meridian-core at $(CORE); set CORE=<path>" >&2; exit 1; }
	@$(DOCKER) build -q -f Dockerfile.rust --target e2e -t meridian-cli-e2e:local . >/dev/null
	@$(MAKE) --no-print-directory -C "$(CORE)" e2e-cluster E2E_DRIVER=cli E2E_CLI_IMAGE=meridian-cli-e2e:local

install-hooks:
	@git config core.hooksPath hooks
	@echo "hooks installed: git push now runs 'make ci-local' first"
