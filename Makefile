# Local npm cache: keep all npm/npx writes inside the project so builds run
# under restricted file sandboxes (e.g. dsh workspace-write) without
# privilege escalation. Existing env vars take precedence (?=).
NPM_CONFIG_CACHE ?= $(PWD)/frontend/.npm-cache
export NPM_CONFIG_CACHE

all: db-migrate frontend css

clean:
	rm -rf target/*
	rm -f dist/css/output.css
	rm -rf dist/admin/
	rm -rf frontend/node_modules/
	rm -rf frontend/.svelte-kit/
	rm -rf frontend/.npm-cache/

css:
	npx @tailwindcss/cli -i ./static/css/input.css -o ./dist/css/output.css

frontend-install:
	cd frontend && npm install

frontend:
	cd frontend && npm run build

sqlx-setup:
	cargo install sqlx-cli
	sqlx database create
	# `sqlx migrate run` cannot bootstrap a FRESH database (see src/db.rs);
	# on an existing database it works. For fresh installs use `make db-migrate`.
	sqlx migrate run --source migrations

# Fresh databases cannot be migrated by the sqlx CLI (see src/db.rs): the
# binary bootstraps the schema itself.
db-migrate: build
	export DATABASE_URL=sqlite://data/db.sqlite
	test -e data/db.sqlite || mkdir -p data && touch data/db.sqlite
	JWT_SECRET=$${JWT_SECRET:-local-dev-secret-0123456789-abcdefghijklmnopqrstuvwxyz} \
	GOOGLE_CLIENT_ID=$${GOOGLE_CLIENT_ID:-local} \
	GOOGLE_CLIENT_SECRET=$${GOOGLE_CLIENT_SECRET:-local} \
	HARDWIRE_DB_PATH=data/db.sqlite \
	./target/release/hardwire --db-init
	cargo sqlx prepare

build:
	cargo build -r

VERSION ?= $(shell git describe --tags --abbrev=0 2>/dev/null | sed 's/^v//' || echo "dev")
IMAGE    := pestouille/hardwire

push:
	docker build --platform linux/amd64 \
		-t $(IMAGE):$(VERSION) \
		-t $(IMAGE):latest \
		.
	docker push $(IMAGE):$(VERSION)
	docker push $(IMAGE):latest

deploy:
	ssh orion 'cd /opt/apps/services && IMAGE_TAG=$(VERSION) docker compose pull hardwire && docker compose up -d hardwire'

tag:
	@test -n "$(V)" || (echo "Usage: make tag V=1.2.3"; exit 1)
	git tag -a v$(V) -m "Release v$(V)"
	git push origin v$(V)
	@echo "Tagged v$(V) and pushed — GitHub Actions will build and deploy"

test:
	DATABASE_URL=sqlite://data/db.sqlite cargo test
