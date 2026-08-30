class Skillroster < Formula
  desc "Local skill governance for AI agents"
  homepage "https://github.com/tt-a1i/skillroster"
  url "https://github.com/tt-a1i/skillroster.git",
      revision: "7df4c68e5e37444a7a701c735e19f2bb3e4b39ec"
  version "1.8.40"
  license "Apache-2.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "skillroster 1.8.40", shell_output("#{bin}/skillroster --version")
    assert_match "One library. The right roster for every agent.", shell_output("#{bin}/skillroster --help")
  end
end
