class Skillroster < Formula
  desc "Local skill governance for AI agents"
  homepage "https://github.com/tt-a1i/skillroster"
  url "https://github.com/tt-a1i/skillroster.git",
      revision: "f8d98f2c2cea117d1f651f5393763aa8244edf8b"
  version "1.8.25"
  license "Apache-2.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "skillroster 1.8.25", shell_output("#{bin}/skillroster --version")
    assert_match "One library. The right roster for every agent.", shell_output("#{bin}/skillroster --help")
  end
end
