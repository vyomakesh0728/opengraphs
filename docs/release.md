# OpenGraphs release flow

This repo publishes versioned binaries (`ogtui`, `ogd`) from git tags using `.github/workflows/release.yml`.

## 1. Bump versions

Update these files together:

- `crates/ogtui/Cargo.toml`
- `crates/ogd/Cargo.toml`
- `pyproject.toml`
- `python/agent-chat/pyproject.toml`

## 2. Commit + tag

```bash
git add -A
git commit -m "Release v0.1.1"
git tag v0.1.1
git push origin main
git push origin v0.1.1
```

## 3. GitHub Actions publishes assets

On tag push, CI builds and uploads:

- `opengraphs-v0.1.1-x86_64-unknown-linux-gnu.tar.gz`
- `opengraphs-v0.1.1-x86_64-apple-darwin.tar.gz`
- `opengraphs-v0.1.1-aarch64-apple-darwin.tar.gz`
- matching `.sha256` files

## 4. User install/update

Users can install/update to latest:

```bash
curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash
```

Or pin a version:

```bash
curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash -s -- --version v0.1.1
```

## 5. Package-manager path

Formula file: `Formula/opengraphs.rb`

Homebrew:

```bash
brew tap vyomakesh0728/opengraphs
brew install vyomakesh0728/opengraphs/opengraphs
brew upgrade vyomakesh0728/opengraphs/opengraphs
```

ZeroBrew:

```bash
# requires zb >= 0.1.2
zb install vyomakesh0728/opengraphs/opengraphs
# zb has no dedicated upgrade command yet; rerun install to refresh
zb install vyomakesh0728/opengraphs/opengraphs
```

Unqualified install (`brew install opengraphs`) requires a Homebrew core merge.
