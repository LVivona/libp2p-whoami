FROM rust:latest

# Set the working directory for your app
WORKDIR /usr/src/node

# Copy the current directory contents into the container at the working directory
COPY . .

# Build the project (this will compile the binary)
RUN cargo build --release

ENV RUST_LOG=INFO
# Specify the command to run the app
CMD ["cargo", "run", "--release", "--", "node", "-f", "luca", "-l", "vivona"]
