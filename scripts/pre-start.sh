#!/bin/bash

# Pre-start script for Entanglement
# Automatically generates JWT_SECRET if not present

set -e

# Run JWT setup if needed
if [ ! -f ".env" ] || ! grep -q "^JWT_SECRET=" ".env"; then
    echo "Setting up JWT secret..."
    ./scripts/setup-jwt.sh
fi

echo "Environment is ready!"