all: index.html index.html.gz target/release/minoru-fediverse-crawler instance_states.svg

index.html: index.md header.html
	pandoc --to html5 --output index.html --include-in-header header.html --standalone index.md

index.html.gz: index.html
	gzip --force --best --keep index.html

# Cargo takes care of rebuilding this file when necessary; we just have to
# force Make to run Cargo even if Make doesn't see a reason to run it.
target/release/minoru-fediverse-crawler: FORCE
	podman build \
		--tag=localhost/minoru-fediverse-crawler-buildhost \
		--file=docker/buildhost.dockerfile \
		docker
	podman run \
		--rm \
		--mount type=bind,src=$(PWD),target=/src \
		--mount type=bind,src=$(HOME)/.cargo/registry,target=/root/.cargo/registry \
		localhost/minoru-fediverse-crawler-buildhost:latest \
		cargo clippy --all-features --all-targets
	podman run \
		--rm \
		--mount type=bind,src=$(PWD),target=/src \
		--mount type=bind,src=$(HOME)/.cargo/registry,target=/root/.cargo/registry \
		localhost/minoru-fediverse-crawler-buildhost:latest \
		cargo test
	podman run \
		--rm \
		--mount type=bind,src=$(PWD),target=/src \
		--mount type=bind,src=$(HOME)/.cargo/registry,target=/root/.cargo/registry \
		localhost/minoru-fediverse-crawler-buildhost:latest \
		cargo build --release

deploy: index.html index.html.gz ansible/minoru-fediverse-crawler.service target/release/minoru-fediverse-crawler
	ansible-playbook --ask-become-pass ansible/deploy.yml

instance_states.svg: instance_states.odg
	libreoffice --headless --convert-to svg:draw_svg_Export $<

plot-checks-histogram:
	sqlite3 minoru-fediverse-crawler.db 'select next_check_datetime from instances' > data.csv
	python3 plot_checks_histogram.py
	rm data.csv

clean:
	cargo clean
	rm -f index.html

FORCE:

.PHONY: deploy clean FORCE
