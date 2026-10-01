cask "oxplay" do
  version "0.1.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/ViceVerse-cz/oxplay/releases/download/v#{version}/oxplay-v#{version}-macOS-ARM64.zip"
  name "Oxplay"
  desc "Experimental native YouTube client"
  homepage "https://github.com/ViceVerse-cz/oxplay"

  depends_on arch: :arm64
  depends_on macos: ">= :tahoe"

  app "Oxplay.app"
end
