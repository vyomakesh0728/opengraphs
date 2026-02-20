class Opengraphs < Formula
  desc "Local-first, TUI-native experiment tracking for AI runs over SSH"
  homepage "https://github.com/vyomakesh0728/opengraphs"
  url "https://github.com/vyomakesh0728/opengraphs/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "86c1f7966a761884b239bd2eb841c6a3f88f870265d5efd92512ae7febdd140c"
  license "NOASSERTION"
  head "https://github.com/vyomakesh0728/opengraphs.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--path", "crates/ogtui", "--root", prefix
    system "cargo", "install", "--locked", "--path", "crates/ogd", "--root", prefix
  end

  test do
    assert_match "opengraphs TUI", shell_output("#{bin}/ogtui --help")
  end
end
