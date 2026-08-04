#!/bin/bash
set -e

# Configuration
NEW_VERSION="${1:-0.0.4}"
CODE_SIGN_IDENTITY="${2}"
GITHUB_TOKEN="${3}"

if [[ -z "$CODE_SIGN_IDENTITY" ]]; then
    echo "❌ Error: Code signing identity is required"
    echo "Usage: NOTARYTOOL_PROFILE=<profile> $0 <version> <code_sign_identity> [github_token]"
    echo "Example: NOTARYTOOL_PROFILE=MyNotaryProfile $0 0.0.4 \"Developer ID Application: Your Name (TEAM_ID)\" ghp_xxxxx"
    exit 1
fi

if [[ -z "${NOTARYTOOL_PROFILE:-}" ]]; then
    echo "Error: NOTARYTOOL_PROFILE is required"
    echo "Create a notarytool keychain profile, then export its name before running this script."
    exit 1
fi

if [[ -z "$GITHUB_TOKEN" ]]; then
    echo ""
    echo "⚠️  GitHub token not found in environment or arguments"
    echo ""
    read -p "Enter GitHub token (or press Enter to skip GitHub release): " INPUT_TOKEN
    if [[ -n "$INPUT_TOKEN" ]]; then
        GITHUB_TOKEN="$INPUT_TOKEN"
        echo "✅ GitHub token provided"
    else
        echo ""
        echo "⚠️  WARNING: Proceeding without GitHub token"
        echo "   - Git tag will be created and pushed"
        echo "   - GitHub release will NOT be created automatically"
        echo "   - DMG will NOT be uploaded to GitHub"
        echo ""
        read -p "Continue without GitHub release? (y/N): " CONTINUE
        if [[ "$CONTINUE" != "y" && "$CONTINUE" != "Y" ]]; then
            echo "❌ Aborted by user"
            exit 1
        fi
    fi
else
    echo "✅ Using GitHub token from environment variable"
fi

echo ""
echo "🚀 Making release for Dictator v${NEW_VERSION}"
echo "   Code signing identity: ${CODE_SIGN_IDENTITY}"
if [[ -n "$GITHUB_TOKEN" ]]; then
    echo "   GitHub release: ✅ Enabled"
else
    echo "   GitHub release: ❌ Disabled (no token)"
fi
echo ""

# Update canonical Tauri and workspace package versions.
echo "📝 Updating Dictator version to ${NEW_VERSION}..."
python3 - "${NEW_VERSION}" <<'PY'
import json
import re
import sys
from pathlib import Path

version = sys.argv[1]
cargo_path = Path("apps/desktop/src-tauri/Cargo.toml")
cargo_content = cargo_path.read_text()
updated, count = re.subn(
    r'(\[package\]\nname = "dictator"\nversion = ")[^"]+',
    rf'\g<1>{version}',
    cargo_content,
    count=1,
)
if count != 1:
    raise SystemExit("could not update Dictator package version")
cargo_path.write_text(updated)

for package_path in (Path("package.json"), Path("apps/desktop/package.json")):
    package = json.loads(package_path.read_text())
    package["version"] = version
    package_path.write_text(json.dumps(package, indent=2) + "\n")
PY
echo "✅ Updated Dictator package versions to ${NEW_VERSION}"

# Clean previous release bundles
echo "🧹 Cleaning previous release bundles..."
rm -rf target/release/bundle
rm -f Dictator.dmg
rm -f Dictator.dmg.sha256
rm -f Dictator.app.dSYM.zip

# Use the existing notarize_app.sh script to build, sign, and notarize
echo "🔨 Building, signing and notarizing with notarize_app.sh..."
if [[ ! -f "./notarize_app.sh" ]]; then
    echo "❌ notarize_app.sh not found!"
    exit 1
fi

chmod +x ./notarize_app.sh
./notarize_app.sh "${CODE_SIGN_IDENTITY}"

if [[ $? -ne 0 ]]; then
    echo "❌ Build/notarization failed!"
    exit 1
fi

echo "✅ Build and notarization successful!"

DMG_PATH="./Dictator.dmg"

# Verify DMG exists
if [[ ! -f "$DMG_PATH" ]]; then
    echo "❌ DMG not found at $DMG_PATH"
    exit 1
fi

# Find and prepare Rust dSYM output when enabled.
DSYM_PATH="./target/release/dictator.dSYM"
DSYM_ZIP_PATH="./Dictator.app.dSYM.zip"

if [[ -d "$DSYM_PATH" ]]; then
    echo "📦 Creating dSYM zip..."
    ditto -c -k --keepParent "$DSYM_PATH" "$DSYM_ZIP_PATH"
    echo "✅ dSYM zip created: $DSYM_ZIP_PATH"
else
    echo "⚠️ dSYM not found at $DSYM_PATH - skipping dSYM upload"
    DSYM_ZIP_PATH=""
fi

# # Generate SHA256
echo "🔍 Generating SHA256..."
shasum -a 256 "$DMG_PATH" > "${DMG_PATH}.sha256"
SHA256=$(cat "${DMG_PATH}.sha256" | cut -d' ' -f1)
echo "SHA256: $SHA256"

# Commit version changes
echo "📝 Committing version changes..."
git add package.json apps/desktop/package.json apps/desktop/src-tauri/Cargo.toml Cargo.lock
git commit -m "Bump version to ${NEW_VERSION}" || echo "No changes to commit"

# Create git tag
echo "🏷️ Creating git tag..."
git tag -a "${NEW_VERSION}" -m "Release ${NEW_VERSION}"

# Push tag to origin
echo "📤 Pushing tag to origin..."
git push origin "${NEW_VERSION}"

if [[ $? -ne 0 ]]; then
    echo "❌ Failed to push tag!"
    exit 1
fi

# Create GitHub release and upload DMG if token is provided
if [[ -n "$GITHUB_TOKEN" ]]; then
    echo "🚀 Creating GitHub release..."
    
    # Create release
    RELEASE_RESPONSE=$(curl -s -L -X POST \
        -H "Accept: application/vnd.github+json" \
        -H "Authorization: Bearer ${GITHUB_TOKEN}" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        https://api.github.com/repos/michael-berardi/dictator/releases \
        -d '{
            "tag_name": "'${NEW_VERSION}'",
            "target_commitish": "master",
            "name": "Release '${NEW_VERSION}'",
            "body": "## Dictator '${NEW_VERSION}'\n\nReal-time audio transcription for macOS using Whisper.\n\n## Installation\n\n### Homebrew (Recommended)\n```bash\nbrew update\nbrew install dictator\n```\n\n### Manual Installation\n1. Download the `Dictator.dmg` file below\n2. Open the DMG and drag Dictator to Applications\n3. Launch the app and grant necessary permissions\n\n## Requirements\n- macOS 14.0 (Sonoma) or later\n- Apple Silicon (ARM64) Mac",
            "draft": false,
            "prerelease": false,
            "generate_release_notes": false
        }')
    
    # Extract release ID from response
    RELEASE_ID=$(echo "$RELEASE_RESPONSE" | grep -o '"id": [0-9]*' | head -1 | grep -o '[0-9]*')
    
    if [[ -z "$RELEASE_ID" ]]; then
        echo "❌ Failed to create GitHub release or extract release ID"
        echo "Response: $RELEASE_RESPONSE"
        exit 1
    fi
    
    echo "✅ GitHub release created (ID: $RELEASE_ID)!"
    echo "📤 Uploading DMG..."
    
    # Upload DMG using the correct API format
    UPLOAD_RESPONSE=$(curl -s -L -X POST \
        -H "Accept: application/vnd.github+json" \
        -H "Authorization: Bearer ${GITHUB_TOKEN}" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        -H "Content-Type: application/octet-stream" \
        "https://uploads.github.com/repos/michael-berardi/dictator/releases/${RELEASE_ID}/assets?name=Dictator.dmg" \
        --data-binary @"${DMG_PATH}")
    
    # Check if upload was successful
    if [[ $(echo "$UPLOAD_RESPONSE" | grep -c '"state":"uploaded"') -gt 0 ]] || [[ $(echo "$UPLOAD_RESPONSE" | grep -c '"state": "uploaded"') -gt 0 ]]; then
        echo "✅ DMG uploaded successfully!"
        # Extract download URL
        DOWNLOAD_URL=$(echo "$UPLOAD_RESPONSE" | grep -o '"browser_download_url":"[^"]*' | cut -d'"' -f4)
        echo "📥 Download URL: $DOWNLOAD_URL"
    elif [[ $(echo "$UPLOAD_RESPONSE" | grep -c '"message"') -gt 0 ]]; then
        echo "❌ Failed to upload DMG"
        echo "Error: $(echo "$UPLOAD_RESPONSE" | grep -o '"message":"[^"]*' | cut -d'"' -f4)"
        exit 1
    else
        echo "⚠️ Upload response unclear, but no error detected"
        echo "Response: $UPLOAD_RESPONSE"
    fi
    
    # Upload dSYM if available
    if [[ -n "$DSYM_ZIP_PATH" && -f "$DSYM_ZIP_PATH" ]]; then
        echo "📤 Uploading dSYM..."
        
        DSYM_UPLOAD_RESPONSE=$(curl -s -L -X POST \
            -H "Accept: application/vnd.github+json" \
            -H "Authorization: Bearer ${GITHUB_TOKEN}" \
            -H "X-GitHub-Api-Version: 2022-11-28" \
            -H "Content-Type: application/zip" \
            "https://uploads.github.com/repos/michael-berardi/dictator/releases/${RELEASE_ID}/assets?name=Dictator.app.dSYM.zip" \
            --data-binary @"${DSYM_ZIP_PATH}")
        
        # Check dSYM upload
        if [[ $(echo "$DSYM_UPLOAD_RESPONSE" | grep -c '"state":"uploaded"') -gt 0 ]] || [[ $(echo "$DSYM_UPLOAD_RESPONSE" | grep -c '"state": "uploaded"') -gt 0 ]]; then
            echo "✅ dSYM uploaded successfully!"
            # Extract download URL
            DSYM_DOWNLOAD_URL=$(echo "$DSYM_UPLOAD_RESPONSE" | grep -o '"browser_download_url":"[^"]*' | cut -d'"' -f4)
            echo "📥 dSYM Download URL: $DSYM_DOWNLOAD_URL"
        elif [[ $(echo "$DSYM_UPLOAD_RESPONSE" | grep -c '"message"') -gt 0 ]]; then
            echo "⚠️ Failed to upload dSYM (non-critical)"
            echo "Error: $(echo "$DSYM_UPLOAD_RESPONSE" | grep -o '"message":"[^"]*' | cut -d'"' -f4)"
        else
            echo "⚠️ dSYM upload response unclear"
        fi
    fi
    
    echo "✅ DMG uploaded successfully!"
    echo "🎉 GitHub release is complete!"
    echo "🔗 Release URL: https://github.com/michael-berardi/dictator/releases/tag/${NEW_VERSION}"
else
    echo "⚠️ Skipping GitHub release creation (no token provided)"
    echo "📋 Manual steps needed:"
    echo "1. Create GitHub release at:"
    echo "   https://github.com/michael-berardi/dictator/releases/new?tag=${NEW_VERSION}"
    echo "2. Upload the DMG file: Dictator.dmg"
fi

echo ""
echo "🎉 Release ${NEW_VERSION} is ready!"
echo ""
echo "📁 Files created:"
echo "   - Dictator.dmg"
echo "   - Dictator.dmg.sha256"
if [[ -f "$DSYM_ZIP_PATH" ]]; then
    echo "   - Dictator.app.dSYM.zip"
fi
echo ""
echo "🍺 Homebrew cask update:"
echo "-----"
cat << EOF
cask "dictator" do
  version "${NEW_VERSION}"
  sha256 "${SHA256}"

  url "https://github.com/michael-berardi/dictator/releases/download/#{version}/Dictator.dmg"
  name "Dictator"
  desc "Private on-device dictation and transcription for macOS"
  homepage "https://github.com/michael-berardi/dictator"

  depends_on macos: ">= :sonoma"
  depends_on arch: :arm64

  app "Dictator.app"

  zap trash: [
    "~/Library/Application Scripts/com.imploselabs.dictator",
    "~/Library/Application Support/com.imploselabs.dictator",
  ]
end
EOF
echo "-----" 