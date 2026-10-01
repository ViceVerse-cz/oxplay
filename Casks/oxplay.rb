cask "oxplay" do
  version "0.1.0"
  sha256 "7e32a38dd288b98f540356f85ac95349c324d85882983355f756f2e7baa8d912"

  url "https://github.com/ViceVerse-cz/oxplay/releases/download/v#{version}/oxplay-v#{version}-macOS-ARM64.zip"
  name "Oxplay"
  desc "Experimental native YouTube client"
  homepage "https://github.com/ViceVerse-cz/oxplay"

  depends_on arch: :arm64
  depends_on macos: ">= :tahoe"

  app "Oxplay.app"
end
