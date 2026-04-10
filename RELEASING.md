# Releasing

## Prerequisites

- Push access to `turadg/git-where` and `turadg/homebrew-tap`
- `CARGO_REGISTRY_TOKEN` secret configured in the repo (for crates.io publish)

## Steps

### 1. Bump version

Update the version in `Cargo.toml`:

```sh
# Edit Cargo.toml version field
cargo check  # ensures Cargo.lock updates
```

Commit and push to `main`:

```sh
git add Cargo.toml Cargo.lock
git commit -m "release: v0.X.0"
git push
```

### 2. Tag and push

```sh
git tag v0.X.0
git push origin v0.X.0
```

This triggers the [release workflow](.github/workflows/release.yml), which:
- Builds binaries for all targets (x86_64/aarch64 × linux/macOS)
- Creates a GitHub release with the tarballs
- Publishes to crates.io

### 3. Wait for the release to complete

Check progress at https://github.com/turadg/git-where/actions

### 4. Update the Homebrew tap

Once the GitHub release is published, update the formula in `turadg/homebrew-tap`.

Download the macOS tarballs and compute their sha256 hashes:

```sh
VERSION=0.X.0
curl -sL "https://github.com/turadg/git-where/releases/download/v${VERSION}/git-where-aarch64-apple-darwin.tar.gz" | shasum -a 256
curl -sL "https://github.com/turadg/git-where/releases/download/v${VERSION}/git-where-x86_64-apple-darwin.tar.gz" | shasum -a 256
```

Edit `Formula/git-where.rb` in the `turadg/homebrew-tap` repo:
1. Update `version` to the new version
2. Replace both `sha256` values with the new hashes

Commit and push:

```sh
git commit -am "git-where v0.X.0"
git push
```

### 5. Verify

```sh
brew update
brew upgrade git-where
git where help
```
