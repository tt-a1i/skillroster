class Skillroster < Formula
  desc "Local skill governance for AI agents"
  homepage "https://github.com/tt-a1i/skillroster"
  url "https://github.com/tt-a1i/skillroster.git",
      revision: "7598ac44e6c67362709592aa17cb7e1477dce72a"
  version "1.0.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "skillroster 1.0.0", shell_output("#{bin}/skillroster --version")
    assert_match "One library. The right roster for every agent.", shell_output("#{bin}/skillroster --help")
  end
end
