PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DESTDIR ?=

CARGO ?= cargo
INSTALL ?= install

BIN := roswire
RELEASE_BIN := target/release/$(BIN)

.PHONY: build install tag cargo-publish-dry-run cargo-publish uninstall integration-local

build:
	$(CARGO) build --release

# Boot a disposable RouterOS CHR container and run the read-only integration
# harness against it. Read-only smoke scope only; requires Docker (e.g. colima
# on macOS). See docs/routeros-local-integration.md.
integration-local:
	./scripts/routeros-local-integration.sh

install: build
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)"
	$(INSTALL) -m 0755 "$(RELEASE_BIN)" "$(DESTDIR)$(BINDIR)/$(BIN)"
	@echo "Installed $(BIN) to $(DESTDIR)$(BINDIR)/$(BIN)"

tag:
	./scripts/tag.sh

cargo-publish-dry-run:
	$(CARGO) publish --dry-run --locked

cargo-publish: cargo-publish-dry-run
	$(CARGO) publish --locked

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN)"
	@echo "Removed $(DESTDIR)$(BINDIR)/$(BIN)"
