PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DESTDIR ?=

CARGO ?= cargo
INSTALL ?= install

BIN := roswire
RELEASE_BIN := target/release/$(BIN)

.PHONY: build install tag uninstall

build:
	$(CARGO) build --release

install: build
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)"
	$(INSTALL) -m 0755 "$(RELEASE_BIN)" "$(DESTDIR)$(BINDIR)/$(BIN)"
	@echo "Installed $(BIN) to $(DESTDIR)$(BINDIR)/$(BIN)"

tag:
	./scripts/tag.sh

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN)"
	@echo "Removed $(DESTDIR)$(BINDIR)/$(BIN)"
