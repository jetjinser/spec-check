SHELL := bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help
CARGO ?= cargo
NIX ?= nix
SPEC_CHECK_NIX ?= ../vostd/nix
CARGO_PRESENT := $(shell command -v $(CARGO) 2>/dev/null)
ifeq ($(strip $(CARGO_PRESENT)),)
RUN ?= $(NIX) develop $(SPEC_CHECK_NIX) --command
else
RUN ?=
endif
CARGO_CMD = $(RUN) $(CARGO)

.PHONY: help fmt fmt-check test clippy check

help: ## Show development checks
	@printf '%s\n' \
		'Usage: make [target]' \
		'' \
		'  fmt        Format the workspace' \
		'  fmt-check  Fail if formatting is stale' \
		'  test       Run workspace tests' \
		'  clippy     Run clippy (warnings reported, not denied)' \
		'  check      fmt-check + test'

fmt:
	$(CARGO_CMD) fmt --all

fmt-check:
	$(CARGO_CMD) fmt --all -- --check

test:
	$(CARGO_CMD) test --workspace

clippy:
	$(CARGO_CMD) clippy --workspace --all-targets

check: fmt-check test
