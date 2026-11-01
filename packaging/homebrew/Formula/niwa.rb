class Niwa < Formula
  desc "Cozy idle garden for your terminal and menu bar"
  homepage "https://github.com/quiinthieu/niwa"
  url "https://github.com/quiinthieu/niwa/archive/refs/tags/v1.0.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/quiinthieu/niwa.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/niwa")
  end

  test do
    ENV["NIWA_HOME"] = (testpath/"niwa-home").to_s
    assert_match version.to_s, shell_output("#{bin}/niwa --version")
    status_json = shell_output("#{bin}/niwa status --json")
    assert_match "\"schema_version\"", status_json
    system bin/"niwa", "peek"
    assert_path_exists testpath/"niwa-home"
  end
end
