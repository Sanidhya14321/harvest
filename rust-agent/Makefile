CARGO ?= cargo

define preserve-lockfiles
	lock_dir=$$(mktemp -d); \
		cp Cargo.lock "$$lock_dir/Cargo.lock"; \
		trap 'cp "$$lock_dir/Cargo.lock" Cargo.lock; rm -rf "$$lock_dir"' EXIT; \
		$(1)
endef

.PHONY: ci hack fmt-check lint test

ci: fmt-check lint test

fmt-check:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy --all-targets --all-features --locked -- -D warnings

test:
	$(CARGO) test --all-features --locked

hack:
	$(call preserve-lockfiles,$(CARGO) fetch --locked && $(CARGO) hack --feature-powerset --no-dev-deps check --offline)
