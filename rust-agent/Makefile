CARGO ?= cargo
PACKAGES = -p harvest-runtime -p harvest-brain -p harvest-omp -p xtask

.PHONY: ci fmt-check lint test
ci: fmt-check lint test
fmt-check:
	$(CARGO) fmt $(PACKAGES) -- --check
lint:
	$(CARGO) clippy $(PACKAGES) --all-targets --all-features --locked -- -D warnings
test:
	$(CARGO) xtask test -p harvest-runtime -p harvest-brain -p harvest-omp --all-features
