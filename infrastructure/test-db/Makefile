COMPOSE := docker compose -f compose.yml
RUN_ARGS := --force-recreate --renew-anon-volumes

.PHONY: postgres postgres-tls certs mysql clickhouse redis postgres-redis all down ps logs

postgres:
	$(COMPOSE) --profile postgres up $(RUN_ARGS) postgres

certs:
	sh postgres-tls/gen-certs.sh

postgres-tls: certs
	$(COMPOSE) --profile postgres-tls up $(RUN_ARGS) --build postgres-tls

mysql:
	$(COMPOSE) --profile mysql up $(RUN_ARGS) mysql

clickhouse:
	$(COMPOSE) --profile clickhouse up $(RUN_ARGS) clickhouse

redis:
	$(COMPOSE) --profile redis up $(RUN_ARGS) redis

postgres-redis:
	$(COMPOSE) --profile postgres --profile redis up $(RUN_ARGS) postgres redis

all:
	$(COMPOSE) --profile postgres --profile mysql --profile clickhouse --profile redis up $(RUN_ARGS)

down:
	$(COMPOSE) --profile postgres --profile postgres-tls --profile mysql --profile clickhouse --profile redis down --volumes --remove-orphans

ps:
	$(COMPOSE) --profile postgres --profile mysql --profile clickhouse --profile redis ps

logs:
	$(COMPOSE) --profile postgres --profile mysql --profile clickhouse --profile redis logs -f
