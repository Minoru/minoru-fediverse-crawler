all: index.html target/release/minoru-fediverse-crawler

index.html: index.md index.css
	pandoc --to html5 --output index.html --include-in-header index.css --standalone index.md

# Cargo takes care of rebuilding this file when necessary; we just have to
# force Make to run Cargo even if Make doesn't see a reason to run it.
target/release/minoru-fediverse-crawler: FORCE
	cargo build --release

deploy: index.html ansible/minoru-fediverse-crawler.service target/release/minoru-fediverse-crawler
	ansible-playbook --ask-become-pass ansible/deploy.yml

clean:
	rm -f index.html

.PHONY: deploy clean

FORCE:
