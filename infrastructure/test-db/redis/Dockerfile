FROM redis/redis-stack-server:7.4.0-v8

COPY seed.redis /seed.redis
COPY seed.sh /usr/local/bin/dbunk-seed-redis

RUN chmod +x /usr/local/bin/dbunk-seed-redis

CMD ["dbunk-seed-redis"]
