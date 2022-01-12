all: css build

clean:
	rm -rf target/*
	rm dist/output.css

css:
	npx tailwindcss -i ./static/css/input.css -o ./dist/output.css

sqlx-setup:
	cargo install sqlx-cli
	sqlx database create
	sqlx migrate --source db/migrations run

db-migrate:
	sqlx migrate --source db/migrations run
	cargo sqlx prepare 

build: css
	cargo sqlx prepare
	cargo build

push:
	docker build -t pestouille/hardwire:0.0.2 .
	docker push pestouille/hardwire:0.0.2
