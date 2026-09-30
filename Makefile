SHELL := /bin/bash

RUST_VERSION := 1.90
DOCKER := DOCKER_BUILDKIT=1 docker

# A pre-push hook runs with GIT_DIR naming this repository, absolutely when
# pushed from a worktree, and the template's `git init`, `fetch` and
# `checkout` in .sdk-scratch would then act on this repository instead:
# staging meridian-python's template/ here and marking this clone shallow.
unexport GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
         GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX

.PHONY: help ci-local ci-local-deep build test lint fmt lock install-hooks e2e-up e2e-migrate \
        vendor-template check-vendored-template check-install check-c-deps

help:
	@echo "  make ci-local   run every gate (the pre-push gate, and what CI mirrors)"
	@echo "  make build      compile the binary"
	@echo "  make test       the unit tests: every check against a fake machine"
	@echo "  make lint       rustfmt --check and clippy with warnings denied"
	@echo "  make check-c-deps  aws-lc-sys is the only crate that compiles C (spec/the-cli, req. 2)"
	@echo "  make check-install  the install script, upgrade and uninstall, against stand-in releases"
	@echo "  make fmt        apply rustfmt"
	@echo "  make lock       regenerate Cargo.lock"
	@echo "  make e2e-up     install into a throwaway namespace and answer the wizard"
	@echo "  make e2e-migrate  plugin migrate, for real, over meridian-python's recorded plugins"
	@echo "  make vendor-template  move the scaffold \`plugin new\` writes to SDK_REV"

# Local green is the completion signal; CI is confirmation.
ci-local: check-vendored-template build test lint check-c-deps check-install
	@echo
	@echo "ci-local: GREEN"

ci-local-deep: ci-local

# What `meridian plugin new` writes: meridian-python's template/, at one SDK
# revision, copied here and compiled into the binary, so scaffolding works
# offline and gives the same plugin every time. The template lives beside the
# SDK it is written against and is tested there against the real sidecar;
# this copy is held to it the way the SDK's bindings are held to the schema.
SDK_REV  := 1d0da36c83a4a354f7a7ef19b3fd402a35b3a7ce
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

check-c-deps:
	@$(DOCKER) build -f Dockerfile.rust --target c-deps . >/dev/null 2>&1 \
		|| { echo "check-c-deps FAILED; see which crate with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target c-deps --progress=plain ." >&2; exit 1; }
	@echo "check-c-deps OK: aws-lc-sys is the only crate that compiles C"

# The install script, shellchecked, and run with `meridian upgrade` and
# `uninstall` against a stand-in for GitHub's releases (spec/the-cli, ruling 8).
check-install:
	@$(DOCKER) build -f Dockerfile.rust --target shellcheck . >/dev/null 2>&1 \
		|| { echo "check-install FAILED: shellcheck; see it with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target shellcheck --progress=plain ." >&2; exit 1; }
	@$(DOCKER) build -f Dockerfile.rust --target install-test . >/dev/null 2>&1 \
		|| { echo "check-install FAILED; see the output with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target install-test --progress=plain --no-cache-filter install-test ." >&2; exit 1; }
	@echo "check-install OK: the install script, upgrade and uninstall, against stand-in releases"

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

# `meridian plugin migrate`, the real binary, over the plugins meridian-python
# records its migrations for, with the steps run in the SDK image built from
# that checkout (its `make base-image`): each must come out as the tree
# meridian-python expects, checked with --run-tests, leaving exactly what it
# expects by hand. Needs meridian-python beside this checkout and a docker
# daemon, whose socket the run is given. Not in ci-local: the steps it runs
# are a sibling's, at whatever that checkout holds.
SDK ?= ../meridian-python
SDK_IMAGE ?= plugin-python:local

e2e-migrate:
	@test -d "$(SDK)/tests/migrations" || { echo "no meridian-python at $(SDK); set SDK=<path>" >&2; exit 1; }
	@$(MAKE) --no-print-directory -C "$(SDK)" base-image BASE_IMAGE=$(SDK_IMAGE)
	@$(DOCKER) build -q -f Dockerfile.rust --target e2e-migrate --build-arg SDK_IMAGE=$(SDK_IMAGE) \
		--build-context fixtures="$(abspath $(SDK))/tests/migrations" -t meridian-cli-e2e-migrate:local . >/dev/null
	@docker run --rm -v /var/run/docker.sock:/var/run/docker.sock meridian-cli-e2e-migrate:local

install-hooks:
	@git config core.hooksPath hooks
	@echo "hooks installed: git push now runs 'make ci-local' first"
