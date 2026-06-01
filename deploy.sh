#!/bin/bash
set -e

IMAGE="hub.umceko.com/byfeb/moneywar"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Versiyon Cargo.toml'dan al
OLD_VERSION=$(grep '^version' "$SCRIPT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

BUMP="patch"
for arg in "$@"; do
  case "$arg" in
    patch|minor|major) BUMP="$arg" ;;
  esac
done

IFS='.' read -r MAJOR MINOR PATCH <<< "$OLD_VERSION"
case "$BUMP" in
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  patch) PATCH=$((PATCH + 1)) ;;
esac
NEW_VERSION="$MAJOR.$MINOR.$PATCH"

# Cargo.toml workspace version güncelle
sed -i "0,/^version = \".*\"/s//version = \"$NEW_VERSION\"/" "$SCRIPT_DIR/Cargo.toml"

echo ">> Version: $OLD_VERSION -> $NEW_VERSION"

# Git commit & tag
git add Cargo.toml Cargo.lock
git commit -m "release: v$NEW_VERSION"
git tag "v$NEW_VERSION"

# Build & push (multi-stage: Rust + Node + runtime)
echo ">> Building Docker image (Rust + frontend)..."
docker build -t "$IMAGE:$NEW_VERSION" -t "$IMAGE:latest" "$SCRIPT_DIR"

echo ">> Pushing image..."
docker push "$IMAGE:$NEW_VERSION"
docker push "$IMAGE:latest"

# K8s deploy
echo ">> Rolling out..."
kubectl set image deployment/moneywar moneywar="$IMAGE:$NEW_VERSION" -n feb
kubectl rollout restart deployment/moneywar -n feb
kubectl rollout status deployment/moneywar -n feb

# Git push
git push && git push --tags

echo ">> Done! v$NEW_VERSION live at https://moneywar.byfeb.com"
