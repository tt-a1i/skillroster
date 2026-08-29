class Skillroster < Formula
  desc "Local skill governance for AI agents"
  homepage "https://github.com/tt-a1i/skillroster"
  url "https://github.com/tt-a1i/skillroster.git",
      revision: "4329cf2a2b476aafa6d77fc6edcfb6b31cd604ed"
  version "1.8.32"
  license "Apache-2.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "skillroster 1.8.32", shell_output("#{bin}/skillroster --version")
    assert_match "One library. The right roster for every agent.", shell_output("#{bin}/skillroster --help")
  end
end
