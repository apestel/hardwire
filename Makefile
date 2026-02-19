all: db-migrate frontend css build

clean:
	rm -rf target/*
	rm -f dist/css/output.css
	rm -rf dist/admin/
	rm -rf frontend/node_modules/
	rm -rf frontend/.svelte-kit/

css:
	npx @tailwindcss/cli -i ./static/css/input.css -o ./dist/css/output.css

frontend-install:
	cd frontend && npm install

frontend:
	cd frontend && npm run build

sqlx-setup:
	cargo install sqlx-cli
	sqlx database create
	sqlx migrate run --source db/migrations

db-migrate:
	export DATABASE_URL=sqlite://data/db.sqlite
	test -e data/db.sqlite || mkdir -p data && touch data/db.sqlite
	sqlx migrate run --source migrations
	cargo sqlx prepare

build: db-migrate
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
