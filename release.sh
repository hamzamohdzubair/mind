#!/bin/bash

# Release script for mind
# Usage: ./release.sh <version>
# Example: ./release.sh 0.1.0-alpha.14

set -e  # Exit on error

if [ -z "$1" ]; then
    echo "Error: Version number required"
    echo "Usage: ./release.sh <version>"
    echo "Example: ./release.sh 0.1.0-alpha.14"
    exit 1
fi

NEW_VERSION="$1"

echo "🚀 Starting release process for version $NEW_VERSION"
echo ""

# Step 1: Update version in Cargo.toml
echo "📝 Step 1: Updating version in Cargo.toml..."
sed -i "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml

# Verify the change
CURRENT_VERSION=$(grep "^version = " Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
if [ "$CURRENT_VERSION" != "$NEW_VERSION" ]; then
    echo "❌ Error: Version update failed"
    exit 1
fi
echo "✅ Version updated to $NEW_VERSION"
echo ""

# Step 2: Run tests
echo "🧪 Step 2: Running tests..."
cargo test --quiet
echo "✅ All tests passed"
echo ""

# Step 3: Build release
echo "🔨 Step 3: Building release..."
cargo build --release --quiet
echo "✅ Release built"
echo ""

# Step 4: Commit version bump
echo "📦 Step 4: Committing version bump..."
git add Cargo.toml
git commit -m "Bump version to $NEW_VERSION

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>"
echo "✅ Version bump committed"
echo ""

# Step 5: Push to GitHub
echo "⬆️  Step 5: Pushing to GitHub..."
git push
echo "✅ Pushed to GitHub"
echo ""

# Step 6: Publish to crates.io
echo "📤 Step 6: Publishing to crates.io..."
cargo publish
echo "✅ Published to crates.io"
echo ""

# Step 7: Install locally
echo "💻 Step 7: Installing locally..."
cargo install mind --version "$NEW_VERSION" --force
echo "✅ Installed locally"
echo ""

# Verify installation
INSTALLED_VERSION=$(mind --version | awk '{print $2}')
if [ "$INSTALLED_VERSION" != "$NEW_VERSION" ]; then
    echo "⚠️  Warning: Installed version ($INSTALLED_VERSION) doesn't match expected ($NEW_VERSION)"
else
    echo "✅ Installation verified"
fi

echo ""
echo "🎉 Release $NEW_VERSION complete!"
echo ""
echo "Next steps:"
echo "  • Test the installation: mind --version"
echo "  • Check on crates.io: https://crates.io/crates/mind"
echo "  • View on GitHub: https://github.com/hamzamohdzubair/mind"
