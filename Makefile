SHELL := /bin/bash

RUST_VERSION := 1.90
DOCKER := DOCKER_BUILDKIT=1 docker

# A pre-push hook runs with GIT_DIR naming this repository, absolutely when
# pushed from a worktree, and the template's `git init`, `fetch` and
# `checkout` in .sdk-scratch would then act on this repository instead:
# staging meridian-python's template/ here and marking this clone shallow.
unexport GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
         GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX

.PHONY: help ci-local ci-local-deep ci-stage pre-push pre-push-run build test lint fmt lock install-hooks e2e-up e2e-migrate \
        vendor-template check-vendored-template check-install check-c-deps check-scaffolds \
        check-fresh-sources smoke-linux \
        ci-mirror-check e2e-authority-macos advisories

help:
	@echo "  make pre-push   the fast gate the pre-push hook runs: lint, build, tests, the stages the changed paths select"
	@echo "  make ci-local   every gate, as CI runs it: optional, for when CI is down"
	@echo "  make ci-stage STAGE=<gate>  one of CI's gates, as CI runs it (ci-local's: $(CI_STAGES))"
	@echo "  make build      compile the binary"
	@echo "  make test       the unit tests: every check against a fake machine"
	@echo "  make lint       rustfmt --check and clippy with warnings denied"
	@echo "  make advisories  no published vulnerability (RustSec) reaches the binary unexplained"
	@echo "  make check-c-deps  aws-lc-sys is the only crate that compiles C (spec/the-cli, req. 2)"
	@echo "  make check-fresh-sources  the target cache's wrapper gives each source the time its content was first seen"
	@echo "  make check-install  the install script, upgrade and uninstall, against stand-in releases"
	@echo "  make check-scaffolds  each scaffold plugin new writes, its tests and suite run, a float and a send on a pause caught"
	@echo "  make smoke-linux  the static Linux binary in each distribution of the supported window: --version, status, plugin check"
	@echo "  make fmt        apply rustfmt"
	@echo "  make lock       regenerate Cargo.lock"
	@echo "  make e2e-up     install into a throwaway namespace and answer the wizard"
	@echo "  make e2e-migrate  plugin migrate, for real, over meridian-python's recorded plugins"
	@echo "  make e2e-authority-macos  authority trust and uninstall on this Mac, for real (needs MERIDIAN_E2E_CHANGES_THIS_MAC=yes)"
	@echo "  make ci-mirror-check  every CI job is reachable from ci-local, or allowlisted with a reason"
	@echo "  make vendor-template  move the scaffolds \`plugin new\` writes to SDK_REV"

# Every gate. CI on the pull request runs it, and the pull request merges on
# it (accepted 2026-11-07, superseding ruling 1075's full local gate); here it
# is the optional full run, for when CI is down.
ci-local: ci-mirror-check check-vendored-template advisories build test lint smoke-linux \
          check-c-deps check-fresh-sources check-install check-scaffolds
	@echo
	@echo "ci-local: GREEN"

ci-local-deep: ci-local

# One of CI's gates, as CI's ci-local job runs it: reproduce a failing one
# with this before pushing again, rather than rerunning CI until it is green.
# ci-local's prerequisites, the same list (ci-mirror-check reads that line).
CI_STAGES := ci-mirror-check check-vendored-template advisories build test lint smoke-linux \
          check-c-deps check-fresh-sources check-install check-scaffolds
ci-stage:
	@case " $(CI_STAGES) " in *" $(STAGE) "*) ;; \
		*) echo "ci-stage: STAGE is one of CI's gates: $(CI_STAGES)" >&2; exit 2;; esac
	@$(MAKE) --no-print-directory $(STAGE)

# The fast gate (accepted 2026-11-07; the pre-push hook runs it): CI runs
# every gate on the pull request, so a push runs only what its changes reach.
# Every push runs the mirror and RustSec's advisories; the rest only when a
# path the branch changed against PRE_PUSH_BASE maps to them here
# (<glob>=<stage>,...; tools/pre_push.py prints which ran and why):
#
#   rust         lint (rustfmt --check, clippy), build and the unit tests:
#                the CLI is one crate, so its tests are the changed crate's
#   vendored     check-vendored-template: the scaffolds are the SDK's
#   c-deps       check-c-deps: aws-lc-sys alone compiles C
#   fresh        check-fresh-sources: the target cache's wrapper
#   install      check-install: the install script, upgrade and uninstall
#   scaffolds    check-scaffolds: each scaffold made by the release binary
#   smoke        smoke-linux: the static binary on each supported system
PRE_PUSH_BASE ?= origin/main
PRE_PUSH_ALWAYS := checks
PRE_PUSH_ORDER := checks rust vendored c-deps fresh install scaffolds smoke
PRE_PUSH_MAP := \
	src/*=rust \
	tests/*=rust \
	build.rs=rust \
	Cargo.toml=rust,c-deps \
	Cargo.lock=rust,c-deps \
	Dockerfile.rust=rust,c-deps,fresh,install,scaffolds,smoke \
	plugin-template/*=rust,vendored,scaffolds \
	plugin-templates/*=rust,vendored,scaffolds \
	sdk-roles/*=rust,vendored,scaffolds \
	src/check.rs=scaffolds src/check/*=scaffolds \
	src/plugin.rs=scaffolds src/plugin/*=scaffolds \
	tests/scaffolds/*=scaffolds \
	tools/fresh-sources.sh=fresh \
	install.sh=install tests/install/*=install \
	tests/smoke/*=smoke \
	.github/workflows/release.yaml=smoke \
	Makefile=vendored,scaffolds,smoke

pre-push:
	@$(PY) tools/pre_push.py --self-test >/dev/null
	@mkdir -p target
	@$(PY) tools/pre_push.py plan --base "$(PRE_PUSH_BASE)" --always "$(PRE_PUSH_ALWAYS)" \
		--order "$(PRE_PUSH_ORDER)" --map "$(PRE_PUSH_MAP)" --out target/pre-push.plan
	@began=$$(date +%s); \
	eval "$(MAKE) --no-print-directory pre-push-run $$(cat target/pre-push.plan)" \
		|| { echo "pre-push: FAILED after $$(( $$(date +%s) - began ))s" >&2; exit 1; }; \
	took=$$(( $$(date +%s) - began )); \
	echo; echo "pre-push: GREEN in $$((took / 60))m$$(printf %02d $$((took % 60)))s; CI runs everything else"

pre-push-run: $(addprefix pre-push-,$(PRE_PUSH_STAGES))

pre-push-checks: ci-mirror-check advisories
pre-push-rust: lint build test
pre-push-vendored: check-vendored-template
pre-push-c-deps: check-c-deps
pre-push-fresh: check-fresh-sources
pre-push-install: check-install
pre-push-scaffolds: check-scaffolds
pre-push-smoke: smoke-linux

# Every job in .github/workflows is reachable from ci-local, or listed in
# tools/ci-mirror-allowlist.txt with the reason it cannot be. The tool is
# meridian-design's, copied byte for byte.
PY := python3

ci-mirror-check:
	@$(PY) tools/ci_mirror_check.py --repo-root .

# What `meridian plugin new` writes: meridian-python's template/, at one SDK
# revision, copied here and compiled into the binary, so scaffolding works
# offline and gives the same plugin every time; and with `--role`, that role's
# template from its templates/, copied to plugin-templates/ the same way. The
# templates live beside the SDK they are written against and are tested there;
# this copy is held to them the way the SDK's bindings are held to the schema.
#
# With them, the SDK's role modules that declare pages on a plugin's behalf
# (a helper such as `custody.account_pages`, decorating `@pages.page(...)` and
# `@pages.route(...)` inside it), every file under src/meridian/roles holding
# such a decorator, copied to sdk-roles/ at their paths there: `role-pages`
# reads the pages and routes each helper declares from them, so a plugin
# calling one is held to what the SDK at SDK_REV declares, not a typed list.
# meridian-python v0.35.0 (contract v29), its main with PR #37 merged: each
# role one generated class, the plugin entered through `meridian.run(...)`
# (rulings 1009-1023). Its template/, templates/ and src/meridian/roles are
# the same trees as at branch 0.35's 2e90655.
SDK_REV  := ab749b77e4274ee7546f3c23789af40ad39ffb2d
SDK_REPO := https://github.com/open-meridian/meridian-python.git
SCRATCH  := .sdk-scratch
SDK_ROLES := $(SCRATCH)/src/meridian/roles

# The template, the templates and src/meridian/roles at SDK_REV in
# $(SCRATCH), and the role modules declaring pages staged in
# $(SCRATCH)/sdk-roles.
define fetch_template
	rm -rf $(SCRATCH) && git init -q $(SCRATCH) \
	&& git -C $(SCRATCH) fetch -q --depth 1 $(SDK_REPO) $(SDK_REV) \
	&& git -C $(SCRATCH) checkout -q FETCH_HEAD -- template templates src/meridian/roles \
	&& mkdir -p $(SCRATCH)/sdk-roles \
	&& for f in $$(grep -rlE '^[[:space:]]*@[A-Za-z_.]*\.(page|route|tool)\(' --include='*.py' $(SDK_ROLES) | sort); do \
		at="$${f#$(SDK_ROLES)/}"; mkdir -p "$(SCRATCH)/sdk-roles/$$(dirname "$$at")" \
		&& cp "$$f" "$(SCRATCH)/sdk-roles/$$at" || exit 1; \
	done
endef

vendor-template:
	@$(fetch_template)
	@rm -rf plugin-template plugin-templates sdk-roles \
		&& cp -R $(SCRATCH)/template plugin-template && cp -R $(SCRATCH)/templates plugin-templates \
		&& cp -R $(SCRATCH)/sdk-roles sdk-roles \
		&& rm -rf $(SCRATCH)
	@echo "vendor-template: plugin-template, plugin-templates and sdk-roles are meridian-python's template, templates and page-declaring role modules at $(SDK_REV)"

check-vendored-template:
	@$(fetch_template) || { echo "check-vendored-template: could not fetch meridian-python at $(SDK_REV)" >&2; rm -rf $(SCRATCH); exit 1; }
	@if diff -r $(SCRATCH)/template plugin-template >/dev/null && diff -r $(SCRATCH)/templates plugin-templates >/dev/null \
		&& diff -r $(SCRATCH)/sdk-roles sdk-roles >/dev/null; then \
		rm -rf $(SCRATCH); echo "check-vendored-template OK: the scaffolds and sdk-roles are meridian-python's at $(SDK_REV)"; \
	else \
		rm -rf $(SCRATCH); echo "check-vendored-template FAILED: plugin-template, plugin-templates or sdk-roles is not meridian-python's at $(SDK_REV). Run 'make vendor-template'." >&2; exit 1; \
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

# Published vulnerabilities in anything the binary links, as meridian-core's
# gate checks them: cargo-deny at the same release against the RustSec
# database, with deny.toml naming every exception and its reason. The stage is
# never taken from the cache, so each run reads the database as it is now and
# an advisory published after the last run of an unchanged Cargo.lock is
# still found. A failure names each advisory found, with its title.
advisories:
	@out=$$($(DOCKER) build -f Dockerfile.rust --target advisories --no-cache-filter advisories --progress=plain . 2>&1) \
		|| { echo "advisories FAILED: a published vulnerability reaches this binary:" >&2; \
		     printf '%s\n' "$$out" \
		       | sed -n -e 's/.*error\[[a-z-]*\]: /title /p' -e 's/.* ID: \(RUSTSEC-[0-9-]*\).*/id \1/p' \
		       | awk '$$1 == "title" { sub(/^title /, ""); t = $$0; next } $$1 == "id" && !seen[$$2]++ { print "  " $$2 ": " t }' >&2; \
		     echo "See it with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target advisories --no-cache-filter advisories --progress=plain ." >&2; exit 1; }
	@echo "advisories OK: no published vulnerability reaches the binary unexplained"

check-c-deps:
	@$(DOCKER) build -f Dockerfile.rust --target c-deps . >/dev/null 2>&1 \
		|| { echo "check-c-deps FAILED; see which crate with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target c-deps --progress=plain ." >&2; exit 1; }
	@echo "check-c-deps OK: aws-lc-sys is the only crate that compiles C"

# tools/fresh-sources.sh (core's, ruling 934), which every cargo run against
# the shared target cache goes through: held to what it promises.
check-fresh-sources:
	@$(DOCKER) build -f Dockerfile.rust --target fresh-sources --no-cache-filter fresh-sources . >/dev/null 2>&1 \
		|| { echo "check-fresh-sources FAILED; see the output with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target fresh-sources --progress=plain --no-cache-filter fresh-sources ." >&2; exit 1; }
	@echo "check-fresh-sources OK: each source is given the time the target cache first saw its content"

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

# Each scaffold `plugin new` writes, made by the release binary, its tests run
# for real against the SDK it pins (installed from meridian-python at SDK_REV):
# the reference plugin, it given custody (every rule, the role's common part as
# advice; role-suite and role-pages failing only when verified), the
# connectivity, custody, data, operations, pricing and reporting templates
# verified, the connectivity, custody, data, pricing and reporting ones with
# their suites, the data template with a float in its
# price path failing, and a connectivity plugin acknowledging a placement after
# a pause on it failing its suite. The check's own tests have no Python.
check-scaffolds:
	@$(DOCKER) build -f Dockerfile.rust --target scaffolds --build-arg SDK_REV=$(SDK_REV) . >/dev/null 2>&1 \
		|| { echo "check-scaffolds FAILED; see the output with:" >&2; \
		     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target scaffolds --build-arg SDK_REV=$(SDK_REV) --progress=plain --no-cache-filter scaffolds ." >&2; exit 1; }
	@echo "check-scaffolds OK: each scaffold keeps the rules, its suite run; a data plugin pricing through a float and a connectivity plugin sending on a pause fail"

# The supported window's Linux systems (README, "Supported systems"; ruling
# 1033): the static Linux binary run in a container of each, where
# tests/smoke/run.sh checks `--version`, `status` against a stand-in kubectl
# and helm, and `plugin new` then `plugin check` on the vendored reference
# plugin. Here it is the binary release.yaml would build for this machine's
# architecture, built in Dockerfile.rust; release.yaml's smoke-linux jobs run
# the same target on each release binary, one system each, with SMOKE_BINARY,
# SMOKE_TARGET and SMOKE_IMAGE, before anything is published. The systems
# are named here and in release.yaml's matrix, and this fails when the two
# differ. A system entering the window is added to both; one leaving it is
# dropped from both.
SMOKE_IMAGES := ubuntu:22.04 ubuntu:26.04 debian:13 rockylinux/rockylinux:9 alpine:3.24
SMOKE_VERSION = $(shell sed -n 's/^version = "\(.*\)"$$/\1/p' Cargo.toml | head -n 1)
SMOKE_BINARY ?=
SMOKE_TARGET ?=
SMOKE_IMAGE ?=

smoke-linux:
	@listed="$$(sed -n 's/^ *image: *\[\(.*\)\] *$$/\1/p' .github/workflows/release.yaml | tr -d '" ' | tr ',' '\n' | sort | tr '\n' ' ')"; \
	 named="$$(printf '%s\n' $(SMOKE_IMAGES) | sort | tr '\n' ' ')"; \
	 [ "$$listed" = "$$named" ] || { echo "smoke-linux FAILED: release.yaml's smoke systems ($$listed) are not the Makefile's ($$named)" >&2; exit 1; }
	@if [ -n "$(SMOKE_BINARY)" ]; then \
		binary="$(abspath $(SMOKE_BINARY))"; target="$(SMOKE_TARGET)"; \
	 else \
		$(DOCKER) build -f Dockerfile.rust --target release-musl-binary --output type=local,dest=target/smoke . >/dev/null 2>&1 \
			|| { echo "smoke-linux FAILED: the static binary did not build; see it with:" >&2; \
			     echo "  DOCKER_BUILDKIT=1 docker build -f Dockerfile.rust --target release-musl-binary --output type=local,dest=target/smoke --progress=plain ." >&2; exit 1; }; \
		binary="$(CURDIR)/target/smoke/meridian"; target="$$(cat target/smoke/target)"; \
	 fi; \
	 chmod 755 "$$binary"; failed=""; \
	 for image in $(or $(SMOKE_IMAGE),$(SMOKE_IMAGES)); do \
		if said="$$(docker run --rm --pull missing -v "$$binary":/usr/local/bin/meridian:ro \
				-v "$(CURDIR)/tests/smoke":/smoke:ro "$$image" sh /smoke/run.sh "$(SMOKE_VERSION)" "$$target" 2>&1)"; then \
			printf '%s\n' "$$said" | sed -n 's/^smoke   /smoke-linux: /p'; \
		else \
			printf '%s\n' "$$said" >&2; failed="$$failed $$image"; \
		fi; \
	 done; \
	 if [ -n "$$failed" ]; then echo "smoke-linux FAILED on:$$failed" >&2; exit 1; fi; \
	 echo "smoke-linux OK: $$target, --version, status and plugin check on $(or $(SMOKE_IMAGE),$(SMOKE_IMAGES))"

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
# cluster in the current kube context, which may hold a deployment of its own:
# the run installs into a namespace of its own, with its registry on a host
# port no pod holds.
#
# The plugin it makes is built on the base image the scaffold names. Between
# moving the scaffold to a new SDK and that SDK's release, which is when this
# has to pass, the base is not published yet, so it is built from
# meridian-python beside this checkout, which must be at that version; once
# published, the release is pulled over any local build of it.
CORE ?= ../meridian-core
TEMPLATE_BASE = $(shell sed -n 's/^ARG BASE=//p' plugin-template/Dockerfile)

e2e-up:
	@test -f "$(CORE)/e2e/cluster/run.py" || { echo "no meridian-core at $(CORE); set CORE=<path>" >&2; exit 1; }
	@if docker manifest inspect "$(TEMPLATE_BASE)" >/dev/null 2>&1; then \
		docker pull -q "$(TEMPLATE_BASE)" >/dev/null; \
	else \
		want="$(lastword $(subst :, ,$(TEMPLATE_BASE)))"; \
		have="$$(sed -n 's/^version = "\(.*\)"$$/\1/p' "$(SDK)/pyproject.toml" 2>/dev/null)"; \
		[ "$$have" = "$$want" ] || { echo "e2e-up: $(TEMPLATE_BASE) is not published, and meridian-python at $(SDK) is $${have:-absent}, not $$want, so it cannot be built here; set SDK=<path>" >&2; exit 1; }; \
		echo "e2e-up: $(TEMPLATE_BASE) is not published yet; building it from $(SDK)"; \
		$(MAKE) --no-print-directory -C "$(SDK)" base-image BASE_IMAGE="$(TEMPLATE_BASE)" || exit 1; \
	fi
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

# `meridian authority trust` on this Mac, for real, as install.sh runs it at a
# terminal, then `uninstall`, and again over a NODE_EXTRA_CA_CERTS already
# named: what the authority-macos workflow runs on a macOS runner. It changes
# this machine's keychain, launchd's environment for apps and its
# LaunchAgents, and puts them back, so it refuses to start without
# MERIDIAN_E2E_CHANGES_THIS_MAC=yes. Here the login keychain is the real one,
# and you answer macOS's password dialog; the CLI's files and binary go in a
# scratch directory. Needs node. Not in ci-local, which runs anywhere and
# changes nothing.
e2e-authority-macos:
	@bash tests/authority-macos/run.sh

install-hooks:
	@git config core.hooksPath hooks
	@echo "hooks installed: git push now runs 'make pre-push' first"
