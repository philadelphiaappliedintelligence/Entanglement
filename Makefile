.PHONY: up down clean setup logs

COMPOSE := docker compose -f server/docker-compose.yml

# Default target
help:
	@echo "Entanglement Docker commands:"
	@echo ""
	@echo "  setup     - Generate JWT_SECRET if needed"
	@echo "  up        - Start all services"
	@echo "  down      - Stop all services"
	@echo "  logs      - Show server logs"
	@echo "  clean     - Remove containers and volumes"
	@echo ""

setup:
	@echo "Setting up environment..."
	@./scripts/pre-start.sh

up: setup
	@echo "Starting Entanglement services..."
	@$(COMPOSE) up -d

down:
	@echo "Stopping Entanglement services..."
	@$(COMPOSE) down

logs:
	@$(COMPOSE) logs -f server

clean:
	@echo "Removing containers and volumes..."
	@$(COMPOSE) down -v
	@docker system prune -f