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

push:
	docker build --platform linux/amd64 -t pestouille/hardwire:0.1.0 .
	docker push pestouille/hardwire:0.1.0

deploy:
	ssh orion 'cd /opt/apps/services && echo `pwd` && docker compose pull && docker compose up -d'
