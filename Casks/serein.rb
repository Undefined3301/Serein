cask "serein" do
  version "1.0.0-nightly.20260918.37"
  sha256 "9546e7d2761998d041613a9ce8f785630c2d80896c059475a42f8baf44abc65c"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
