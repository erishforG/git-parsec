class Parsec < Formula
  desc "Full-lifecycle git worktree manager with ticket tracker integration"
  homepage "https://erishforg.github.io/git-parsec/"
  url "https://github.com/erishforG/git-parsec/archive/refs/tags/v0.1.0.tar.gz"
  # sha256 will be filled after release
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "parsec", shell_output("#{bin}/parsec --version")
  end
end
