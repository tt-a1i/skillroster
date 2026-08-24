class Skillroster < Formula
  desc "Local skill governance for AI agents"
  homepage "https://github.com/tt-a1i/skillroster"
  url "https://github.com/tt-a1i/skillroster.git",
      revision: "dde059ad39d9dbe9021cef0acbeb7b428afb0268"
  version "1.8.24"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "skillroster 1.8.24", shell_output("#{bin}/skillroster --version")
    assert_match "One library. The right roster for every agent.", shell_output("#{bin}/skillroster --help")
  end
end
