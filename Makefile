# GPL-3.0-or-later - see LICENSE file for full terms
# Copyright 2026 Michael Van Auker (HMRDSmoke)
# Do not remove these comments.
# branchkeeper/Makefile
# Makefile

# ===========================================================================
# Branchkeeper — build and install

# Preserve PATH and HOME when running under sudo.
# SUDO_USER is set by sudo to the original user; fall back to USER when not sudo.
SUDO_USER     ?= $(USER)
REAL_HOME     := $(shell getent passwd $(SUDO_USER) | cut -d: -f6)
export PATH   := $(REAL_HOME)/.cargo/bin:$(PATH)
export HOME   := $(REAL_HOME)
#
# make              → debug build
# make release      → optimized release build
# make install      → install binary + desktop file + metainfo + icon (requires sudo)
# make uninstall    → remove installed files
# make clean        → remove build artifacts
# ===========================================================================
PREFIX        ?= /usr/local
BINDIR        := $(PREFIX)/bin
APPDIR        := /usr/share/applications
METADIR       := /usr/share/metainfo
ICON_BASE     := /usr/share/icons/hicolor
ICON_SIZES    := 16 22 24 48 64 128 256
APP_ID        := io.github.hmrdsmoke.Branchkeeper
BIN           := target/release/branchkeeper
DESK          := resources/$(APP_ID).desktop
METAINFO      := resources/$(APP_ID).metainfo.xml
ICONSRC       := resources/icons/hicolor
ICON_SVG      := resources/icon.svg
ICON_SVG_DST  := $(ICON_BASE)/scalable/apps/$(APP_ID).svg

.PHONY: all release install uninstall clean

# ── Default: debug build ──────────────────────────────────────────────────────
all:
	cargo build

# ── Release build ─────────────────────────────────────────────────────────────
release:
	cargo build --release

# ── Install ───────────────────────────────────────────────────────────────────
install: release
	@echo "Installing branchkeeper → $(BINDIR)/$(APP_ID)"
	install -Dm755 $(BIN) $(BINDIR)/$(APP_ID)
	@echo "Installing desktop file → $(APPDIR)"
	install -Dm644 $(DESK) $(APPDIR)/$(APP_ID).desktop
	@echo "Installing metainfo → $(METADIR)"
	install -Dm644 $(METAINFO) $(METADIR)/$(APP_ID).metainfo.xml
	@echo "Installing scalable icon → $(ICON_BASE)/scalable/apps"
	install -Dm644 $(ICON_SVG) $(ICON_SVG_DST)
	@echo "Installing per-size icons (skips sizes not yet drawn)..."
	@for sz in $(ICON_SIZES); do \
		src="$(ICONSRC)/$${sz}x$${sz}/apps/$(APP_ID).png"; \
		if [ -f "$$src" ]; then \
			install -Dm644 "$$src" "$(ICON_BASE)/$${sz}x$${sz}/apps/$(APP_ID).png"; \
			echo "  → $${sz}x$${sz}"; \
		fi; \
	done
	@echo "Updating icon cache and desktop database..."
	gtk-update-icon-cache /usr/share/icons/hicolor 2>/dev/null || true
	update-desktop-database $(APPDIR) 2>/dev/null || true
	@echo ""
	@echo "✓ Branchkeeper installed."
	@echo "  → Open COSMIC Panel settings and add Branchkeeper to your panel."

# ── Uninstall ─────────────────────────────────────────────────────────────────
uninstall:
	@echo "Removing Branchkeeper..."
	rm -f $(BINDIR)/$(APP_ID)
	rm -f $(APPDIR)/$(APP_ID).desktop
	rm -f $(METADIR)/$(APP_ID).metainfo.xml
	rm -f $(ICON_SVG_DST)
	@for sz in $(ICON_SIZES); do \
		rm -f "$(ICON_BASE)/$${sz}x$${sz}/apps/$(APP_ID).png"; \
	done
	gtk-update-icon-cache /usr/share/icons/hicolor 2>/dev/null || true
	update-desktop-database $(APPDIR) 2>/dev/null || true
	@echo "✓ Branchkeeper uninstalled."

# ── Clean ─────────────────────────────────────────────────────────────────────
clean:
	cargo clean
