NAME    := redhat-ai-configurator
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIST    := dist
TOP     := $(DIST)/rpmbuild

.PHONY: all build test check lint tarballs rpm srpm clean

all: build

build:
	cargo build --release --locked

test:
	cargo test --locked

check: test

lint:
	cargo clippy --all-targets -- -D warnings

# Always regenerate: the tarballs have no file prerequisites, so without this
# `make rpm` would happily package a stale tree after a source edit.
tarballs: $(DIST)
	rm -f $(DIST)/$(NAME)-$(VERSION).tar.gz $(DIST)/$(NAME)-$(VERSION)-vendor.tar.gz
	$(MAKE) $(DIST)/$(NAME)-$(VERSION).tar.gz $(DIST)/$(NAME)-$(VERSION)-vendor.tar.gz

$(DIST):
	mkdir -p $(DIST)

$(DIST)/$(NAME)-$(VERSION).tar.gz: $(DIST)
	rm -rf $(DIST)/stage
	mkdir -p $(DIST)/stage/$(NAME)-$(VERSION)
	rsync -a --exclude target --exclude dist --exclude vendor --exclude .cargo \
		--exclude .git --exclude '*~' ./ $(DIST)/stage/$(NAME)-$(VERSION)/
	tar -C $(DIST)/stage -czf $@ $(NAME)-$(VERSION)
	rm -rf $(DIST)/stage

$(DIST)/$(NAME)-$(VERSION)-vendor.tar.gz: $(DIST)
	rm -rf $(DIST)/vendor-stage $(DIST)/vendor $(DIST)/vendor-config.toml
	cargo vendor --locked $(DIST)/vendor > $(DIST)/vendor-config.toml
	sed -i 's|^directory = .*|directory = "vendor"|' $(DIST)/vendor-config.toml
	mkdir -p $(DIST)/vendor-stage/.cargo
	cp $(DIST)/vendor-config.toml $(DIST)/vendor-stage/.cargo/config.toml
	mv $(DIST)/vendor $(DIST)/vendor-stage/vendor
	tar -C $(DIST)/vendor-stage -czf $@ vendor .cargo
	rm -rf $(DIST)/vendor-stage $(DIST)/vendor-config.toml

rpm: tarballs
	mkdir -p $(TOP)/SOURCES $(TOP)/SPECS $(TOP)/BUILD $(TOP)/RPMS $(TOP)/SRPMS
	cp $(DIST)/$(NAME)-$(VERSION).tar.gz $(DIST)/$(NAME)-$(VERSION)-vendor.tar.gz $(TOP)/SOURCES/
	cp packaging/$(NAME).spec $(TOP)/SPECS/
	rpmbuild -ba --define "_topdir $(CURDIR)/$(TOP)" $(TOP)/SPECS/$(NAME).spec

srpm: tarballs
	mkdir -p $(TOP)/SOURCES $(TOP)/SPECS $(TOP)/SRPMS
	cp $(DIST)/$(NAME)-$(VERSION).tar.gz $(DIST)/$(NAME)-$(VERSION)-vendor.tar.gz $(TOP)/SOURCES/
	cp packaging/$(NAME).spec $(TOP)/SPECS/
	rpmbuild -bs --define "_topdir $(CURDIR)/$(TOP)" $(TOP)/SPECS/$(NAME).spec

clean:
	cargo clean
	rm -rf $(DIST)
