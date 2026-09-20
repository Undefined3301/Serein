cask "serein" do
  version "1.0.0-nightly.20260920.41"
  sha256 "7830edfe2737a5f56a81e29e1bf301a6d49c35f5238155b7e084a20cf1e9f538"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
