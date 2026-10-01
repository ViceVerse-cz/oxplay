cask "oxplay" do
  version "0.1.1"
  sha256 "24d0c67ac5c9f8d1a97d820a46fc0825774c83e91d5741add74e8941fa612a78"

  url "https://github.com/ViceVerse-cz/oxplay/releases/download/v#{version}/oxplay-v#{version}-macOS-ARM64.zip"
  name "Oxplay"
  desc "Experimental native YouTube client"
  homepage "https://github.com/ViceVerse-cz/oxplay"

  depends_on arch: :arm64
  depends_on macos: ">= :tahoe"

  app "Oxplay.app"
end
