all: db-migrate css build

clean:
	rm -rf target/*
	rm dist/output.css

css:
	npx @tailwindcss/cli -i ./static/css/input.css -o ./dist/css/output.css

sqlx-setup:
	cargo install sqlx-cli
	sqlx database create
	sqlx migrate run --source db/migrations

db-migrate:
	export DATABASE_URL=sqlite://db/db.sqlite3
	test -e db/db.sqlite3 || echo "" > db/db.sqlite3
	sqlx migrate run --source db/migrations
	cargo sqlx prepare

build: css db-migrate
	cargo build -r

push:
	docker build --platform linux/amd64 -t pestouille/hardwire:0.1.0 .
	docker push pestouille/hardwire:0.1.0

deploy:
	ssh orion 'cd /opt/apps/services && echo `pwd` && docker compose pull && docker compose up -d'
