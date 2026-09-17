cask "serein" do
  version "1.0.0-nightly.20260916.34"
  sha256 "8b38f14ae2dd7eb8b4ccc90a1d8f76a77da8cabcdb7463e4c90b24c8f34b0f82"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
