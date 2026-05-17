class Typekart < Formula
  desc "Terminal typing racer with kart-style item effects"
  homepage "https://github.com/tom-sitter/typekart"
  license "GPL-3.0-or-later"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tom-sitter/typekart/releases/download/vREPLACE_WITH_VERSION/typekart-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ARM64_SHA256"
    else
      url "https://github.com/tom-sitter/typekart/releases/download/vREPLACE_WITH_VERSION/typekart-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_SHA256"
    end
  end

  def install
    bin.install "typekart"
  end

  test do
    assert_match "A terminal typing racer", shell_output("#{bin}/typekart --help")
  end
end
