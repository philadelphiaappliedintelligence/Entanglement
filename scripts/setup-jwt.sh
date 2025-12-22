#!/bin/bash

# Automatic JWT secret setup script
# Generates and persists JWT_SECRET in .env file if not already present

set -e

ENV_FILE=".env"

# Function to generate JWT secret
generate_jwt_secret() {
    openssl rand -hex 32
}

# Check if JWT_SECRET exists in .env
if [ -f "$ENV_FILE" ] && grep -q "^JWT_SECRET=" "$ENV_FILE"; then
    echo "JWT_SECRET already exists in .env"
    exit 0
fi

# Generate new JWT secret
JWT_SECRET=$(generate_jwt_secret)

# Create or update .env file
if [ ! -f "$ENV_FILE" ]; then
    touch "$ENV_FILE"
fi

# Add JWT_SECRET to .env
echo "" >> "$ENV_FILE"
echo "# JWT secret for authentication (auto-generated on $(date))" >> "$ENV_FILE"
echo "JWT_SECRET=$JWT_SECRET" >> "$ENV_FILE"

echo "JWT_SECRET generated and saved to .env"