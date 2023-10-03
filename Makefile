all: css build

clean:
	rm -rf target/*
	rm dist/output.css

css:
	npx tailwindcss -i ./static/css/input.css -o ./dist/css/output.css

sqlx-setup:
	cargo install sqlx-cli
	sqlx database create
	sqlx migrate run --source db/migrations

db-migrate:
	sqlx migrate run --source db/migrations
	cargo sqlx prepare 

build: css db-migrate
	cargo build -r

push:
	docker build -t pestouille/hardwire:0.0.7 .
	docker push pestouille/hardwire:0.0.7

deploy:
	ssh orion 'cd /opt/apps/services && echo `pwd` && docker compose pull && docker compose up -d'
