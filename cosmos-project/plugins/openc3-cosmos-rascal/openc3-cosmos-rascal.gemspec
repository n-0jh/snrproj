# encoding: ascii-8bit

# Create the overall gemspec
Gem::Specification.new do |s|
  s.name = 'openc3-cosmos-rascal'
  s.summary = 'OpenC3 openc3-cosmos-rascal plugin'
  s.description = <<-EOF
    openc3-cosmos-rascal plugin for deployment to OpenC3
  EOF
  s.licenses = 'MIT'
  s.authors = ['Anonymous']
  s.email = ['name@domain.com']
  s.homepage = 'https://github.com/OpenC3/cosmos'
  s.platform = Gem::Platform::RUBY
  s.required_ruby_version = '>= 3.0'

  if ENV['VERSION']
    s.version = ENV['VERSION'].dup
  else
    time = Time.now.strftime("%Y%m%d%H%M%S")
    s.version = '0.0.0' + ".#{time}"
  end
  s.files = Dir.glob("{targets,lib,public,tools,microservices}/**/*") + %w(Rakefile README.md LICENSE.md plugin.txt)

  s.metadata = {
    # These fields are used when you submit your plugin to the OpenC3 Store at store.openc3.com
    # See this help page for more detail: https://store.openc3.com/help/guidelines
    "source_code_uri" => "https://github.com/your-github/plugin-repo",
    "openc3_store_title" => "RASCAL",
    "openc3_store_description" => "Describe what your plugin does here.",
    "openc3_store_keywords" => "some, comma-delimited, search terms",
    "openc3_store_image" => "public/store_img.png",
    "openc3_cosmos_minimum_version" => "6.0.0", # OPTIONAL
    "openc3_store_access_type" => "public" # OPTIONAL
  }
end
