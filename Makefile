all: index.html

index.html: index.md index.css
	pandoc --to html5 --output index.html --include-in-header index.css --standalone index.md

deploy: index.html
	ansible-playbook --ask-become-pass ansible/deploy.yml

clean:
	rm -f index.html

.PHONY: deploy clean
