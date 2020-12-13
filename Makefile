TAG = "$(git describe --tags)"
DANGLING_IMAGES = "$(docker images -q --filter "dangling=true")"

build-api:
	@docker-compose build --build-arg VERSION="$(TAG)" api

up:
	@docker-compose up -d

cli:
	@docker exec -it kalaxia_v2_api sh

clean-images:
	@docker rmi "$(DANGLING_IMAGES)"